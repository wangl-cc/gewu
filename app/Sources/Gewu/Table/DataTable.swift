import AppKit
import SwiftUI

/// SwiftUI updates this bridge frequently. The controller reloads rows only
/// when the window publishes a new immutable View, preserving the live NSTableView.
struct DataTable: NSViewControllerRepresentable {
    @ObservedObject var model: DocumentModel
    // Observed only so SwiftUI calls `update` when Find state or highlights change.
    @ObservedObject private var find: FindModel
    var showsRowNumbers: Bool
    @AppStorage("TableFontSize") private var fontSize = 12.0
    @AppStorage("TableRowHeight") private var rowHeight = 28.0
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    init(model: DocumentModel, showsRowNumbers: Bool) {
        self.model = model
        self.showsRowNumbers = showsRowNumbers
        _find = ObservedObject(wrappedValue: model.find)
    }

    func makeNSViewController(context: Context) -> DataTableController {
        let controller = DataTableController(model: model)
        model.table = controller
        return controller
    }

    func updateNSViewController(_ controller: DataTableController, context: Context) {
        controller.update(
            showsRowNumbers: showsRowNumbers, fontSize: min(max(fontSize, 10), 20),
            rowHeight: min(max(rowHeight, 24), 36), rowNumberBase: rowNumberBase)
    }

    static func dismantleNSViewController(_ controller: DataTableController, coordinator: ()) {
        controller.closePopover()
        controller.stopLoading()
    }
}

@MainActor
final class DataTableController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let model: DocumentModel
    private let table = NSTableView()
    private let scroll = NSScrollView()
    private lazy var gutter = RowGutterView(table: table)
    private let cache = RowCache()
    private let reader = TableRowReader()
    private var pending: [RowCache.Key] = []
    private var loading: RowCache.Key?
    private var loadTask: Task<Void, Never>?
    private var fitTask: Task<Void, Never>?
    private var findRevision: UInt64?
    private var highlightRevision: UInt64?
    private var currentFindMatch: CellMatch?
    private var failedView: ObjectIdentifier?
    private var widths = ColumnWidths()
    private var automaticFitTask: Task<Void, Never>?
    private var fittingAutomatically = false
    private var applyingWidths = false
    private var firstWidthsPending = true
    private let widthWorker = DispatchQueue(label: "gewu.table.widths", qos: .userInitiated)
    private var popover: NSPopover?
    private var builtColumns = false
    private var applyingColumns = false
    private var displayedView: GewuView?
    private var showsNumbers: Bool?
    private var reloading = false
    private var textSize: CGFloat = 12
    private var fonts = CellFonts()
    private var rowNumberBase = 0
    private let rowHeaderID = NSUserInterfaceItemIdentifier("rowHeader")
    private let rowViewID = NSUserInterfaceItemIdentifier("stripedRow")

    init(model: DocumentModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func loadView() {
        let root = NSView()
        let header = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: 32))
        header.onFilterClick = { [weak self] in self?.showFilter(at: $0) }
        header.columnMenu = { [weak self] in self?.columnMenu(at: $0) }

        table.headerView = header
        table.alphaValue = 0
        table.target = self
        table.action = #selector(tableClicked(_:))
        table.usesAlternatingRowBackgroundColors = false
        table.allowsMultipleSelection = true
        table.allowsColumnReordering = true
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.style = .plain
        table.rowHeight = 28
        table.intercellSpacing = .zero
        table.gridStyleMask = []
        table.dataSource = self
        table.delegate = self
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(visibleRectChanged), name: NSView.boundsDidChangeNotification,
            object: scroll.contentView)
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            NotificationCenter.default.addObserver(
                self, selector: #selector(windowActivityChanged), name: name, object: nil)
        }
        // SwiftUI positions this view below its toolbar; no second titlebar inset.
        scroll.automaticallyAdjustsContentInsets = false
        for child in [scroll, gutter] {
            child.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(child)
        }
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: root.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            gutter.topAnchor.constraint(equalTo: root.topAnchor),
            gutter.bottomAnchor.constraint(equalTo: scroll.contentView.bottomAnchor),
            gutter.leadingAnchor.constraint(equalTo: root.leadingAnchor),
        ])
        view = root
    }

    func update(showsRowNumbers: Bool, fontSize: CGFloat = 12, rowHeight: CGFloat = 28, rowNumberBase: Int = 0) {
        loadViewIfNeeded()
        let info = model.info
        let findChanged =
            findRevision != model.find.revision || highlightRevision != model.find.highlightRevision
            || currentFindMatch != model.find.currentMatch
        findRevision = model.find.revision
        highlightRevision = model.find.highlightRevision
        currentFindMatch = model.find.currentMatch
        let effectiveHeight = max(rowHeight, fontSize + 8)
        let presentationChanged =
            textSize != fontSize || table.rowHeight != effectiveHeight || self.rowNumberBase != rowNumberBase
        if presentationChanged {
            widths.fontChanged()
            textSize = fontSize
            fonts = CellFonts(size: fontSize)
            self.rowNumberBase = rowNumberBase
            table.rowHeight = effectiveHeight
            table.headerView?.frame.size.height = max(32, ceil(fontSize * 2.6))
            scroll.tile()
            gutter.font = fonts.rowNumber
        }
        if !builtColumns {
            buildColumns(info)
            builtColumns = true
        }
        if showsNumbers != showsRowNumbers {
            showsNumbers = showsRowNumbers
            gutter.setShown(showsRowNumbers)
            table.tableColumn(withIdentifier: rowHeaderID)?.isHidden = !showsRowNumbers
        }
        if displayedView !== model.view {
            displayedView = model.view
            closePopover()
            stopLoading()
            cache.clear()
            failedView = nil
            reloading = true
            table.deselectAll(nil)
            table.reloadData()
            reloading = false
            gutter.needsDisplay = true
        } else if presentationChanged || findChanged {
            // Recreate visible cells from the existing cache without re-running
            // row formatting when only Find presentation changes.
            reloading = true
            table.reloadData()
            reloading = false
        }
        if presentationChanged {
            fitGutter(info)
            gutter.needsDisplay = true
        }
        applyingColumns = true
        let visible = model.appliedSpec.columns.map(\.column)
        for column in table.tableColumns {
            if let i = columnIndex(column) { column.isHidden = !visible.contains(UInt32(i)) }
        }
        for (offset, column) in visible.enumerated() {
            let from = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
            if from != offset + 1 { table.moveColumn(from, toColumn: offset + 1) }
        }
        applyingColumns = false
        for column in table.tableColumns {
            guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
            header.textSize = textSize
            header.filtered = model.appliedFilterColumns.contains(UInt32(i))
            header.inspected = model.inspectorVisible && model.inspectedColumn == i
            let sort = model.appliedSpec.columns.first { $0.column == UInt32(i) }?.sort
            header.sorted = sort != nil
            header.sortPriority = sort.map { Int($0.priority) + 1 } ?? 0
            header.sortAscending = !(sort?.descending ?? false)
        }
        table.headerView?.needsDisplay = true
        requestHighlights()
        fitVisibleColumns()
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    @objc private func windowActivityChanged(_ notification: Notification) {
        guard let window = notification.object as? NSWindow, window === view.window else { return }
        table.enumerateAvailableRowViews { row, _ in
            for cell in row.subviews.compactMap({ $0 as? MatchCellView }) { cell.refreshAppearance() }
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        fitVisibleColumns()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        fitVisibleColumns()
    }

    @objc private func visibleRectChanged() {
        requestHighlights()
        fitVisibleColumns()
    }

    private func requestHighlights() {
        let rows = table.rows(in: table.visibleRect)
        guard rows.location != NSNotFound, rows.length > 0 else { return }
        model.find.requestHighlight(start: UInt64(rows.location), count: UInt32(rows.length))
    }

    private func buildColumns(_ info: FileInfo) {
        gutter.label = { [weak self] row in
            guard let self, let block = self.block(for: row, column: nil) else { return nil }
            let r = row - Int(block.start)
            return r >= 0 && r < Int(block.nRows) ? formatRowNumber(block.sourceRows[r], base: self.rowNumberBase) : nil
        }
        let rowHeader = NSTableColumn(identifier: rowHeaderID)
        rowHeader.headerCell = ColumnHeaderCell(textCell: "")
        rowHeader.width = gutter.fittedWidth
        rowHeader.minWidth = gutter.fittedWidth
        rowHeader.maxWidth = gutter.fittedWidth
        rowHeader.resizingMask = []
        table.addTableColumn(rowHeader)
        fitGutter(info)
        for (i, column) in info.columns.enumerated() {
            let c = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(i)"))
            let header = ColumnHeaderCell(textCell: column.name)
            header.textSize = textSize
            header.subtitle = column.displayType
            header.showsFunnel = true
            c.headerCell = header
            c.headerToolTip =
                "\(column.name): \(column.technicalType)\nClick to cycle sorting; Shift-click for multiple keys. Use the funnel to filter.\nDrag to reorder; double-click a divider to fit the column."
            c.width = 140
            c.minWidth = 50
            table.addTableColumn(c)
        }
    }

    private func fitGutter(_ info: FileInfo) {
        gutter.fit(largest: rowNumber(max(info.rowCount, 1) - 1, base: rowNumberBase))
        gutter.toolTip = "Row in file (\(formatRowNumber(0, base: rowNumberBase))-based)"
        if let column = table.tableColumn(withIdentifier: rowHeaderID) {
            column.minWidth = gutter.fittedWidth
            column.maxWidth = gutter.fittedWidth
            column.width = gutter.fittedWidth
        }
    }

    private func columnIndex(_ column: NSTableColumn) -> Int? {
        let id = column.identifier.rawValue
        return id.hasPrefix("c") ? Int(id.dropFirst()) : nil
    }

    private func block(for row: Int, column: UInt32?) -> RowBlock? {
        let key = RowCache.Key(start: row / RowCache.blockSize * RowCache.blockSize, column: column)
        if let block = cache.block(for: key) { return block }
        guard failedView != displayedView.map(ObjectIdentifier.init), loading != key, !pending.contains(key) else {
            return nil
        }
        // Bound queued work as well as cached data. Completion revisits visible
        // placeholders, so a full queue never permanently drops a visible cell.
        if pending.count < 128 { pending.append(key) }
        loadNext()
        return nil
    }

    private func loadNext() {
        guard loading == nil, !pending.isEmpty, let view = displayedView else { return }
        let key = pending.removeFirst()
        loading = key
        let reader = reader
        loadTask = Task { [weak self] in
            do {
                let block = try await reader.read(
                    view, start: UInt64(key.start),
                    count: UInt32(RowCache.blockSize), columns: key.column.map { [$0] } ?? [])
                guard !Task.isCancelled, let self, self.displayedView === view, self.model.view === view else { return }
                self.cache.insert(block, for: key)
                self.display(block, column: key.column)
                self.loading = nil
                self.fillVisiblePlaceholders()
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.displayedView === view, self.model.view === view else { return }
                self.loading = nil
                self.pending.removeAll()
                self.failedView = ObjectIdentifier(view)
                self.model.reportError(error, prefix: "Could not load rows: ")
            }
        }
    }

    private func display(_ block: RowBlock, column: UInt32?) {
        if let column {
            let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
            guard index >= 0 else { return }
            for offset in 0..<Int(block.nRows) {
                let row = Int(block.start) + offset
                if let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView {
                    render(cell, column: Int(column), row: row, block: block)
                }
            }

        }
        gutter.needsDisplay = true
        if let row = model.selectedDisplayRow, row >= Int(block.start), row < Int(block.start) + Int(block.nRows) {
            model.selectedSourceRow = block.sourceRows[row - Int(block.start)]
        }
    }

    private func fillVisiblePlaceholders() {
        let rows = table.rows(in: table.visibleRect)
        guard rows.location != NSNotFound else { return }
        for index in table.columnIndexes(in: table.visibleRect) {
            guard let column = columnIndex(table.tableColumns[index]) else { continue }
            for row in rows.location..<(rows.location + rows.length) {
                guard let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView,
                    cell.isLoading
                else { continue }
                if let block = block(for: row, column: UInt32(column)) {
                    render(cell, column: column, row: row, block: block)
                }
            }
        }
    }

    func stopLoading() {
        loadTask?.cancel()
        fitTask?.cancel()
        automaticFitTask?.cancel()
        pending.removeAll()
        loading = nil
    }

    func numberOfRows(in tableView: NSTableView) -> Int { Int(model.view.rowCount()) }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let result = (table.makeView(withIdentifier: rowViewID, owner: self) as? StripedRowView) ?? StripedRowView()
        result.identifier = rowViewID
        if !result.wantsLayer { result.wantsLayer = true }
        result.odd = row % 2 == 1
        return result
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let tableColumn, let col = columnIndex(tableColumn) else { return nil }
        let cell =
            (table.makeView(withIdentifier: tableColumn.identifier, owner: self) as? MatchCellView)
            ?? makeCell(identifier: tableColumn.identifier)
        let kind = model.kinds[col]
        cell.isLoading = true
        cell.configure(text: "…", font: fonts.font(for: kind), alignment: kind.alignment, muted: true)
        if let block = block(for: row, column: UInt32(col)) { render(cell, column: col, row: row, block: block) }
        return cell
    }

    private func render(_ cell: MatchCellView, column: Int, row: Int, block: RowBlock) {
        guard row >= Int(block.start), row < Int(block.start) + Int(block.nRows) else {
            return
        }
        let kind = model.kinds[column]
        let index = row - Int(block.start)
        let hit = model.find.isMatch(row: row, column: column)
        let current = model.find.currentMatch
        cell.isLoading = false
        let text = block.cells[index]
        cell.configure(
            text: text ?? "null", font: text == nil ? fonts.null : fonts.font(for: kind), alignment: kind.alignment,
            query: hit && kind == .text ? model.find.activeQuery : nil, wholeMatch: hit && kind != .text,
            current: current?.displayRow == UInt64(row) && current?.column == UInt32(column), muted: text == nil)
    }

    private func makeCell(identifier: NSUserInterfaceItemIdentifier) -> MatchCellView {
        let cell = MatchCellView()
        cell.identifier = identifier
        // Reused text fields can briefly retain their old width during layout.
        // macOS 14+ views do not clip descendants by default.
        cell.clipsToBounds = true
        let field = NSTextField(labelWithString: "")
        field.clipsToBounds = true
        field.lineBreakMode = .byTruncatingTail
        field.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -10),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    func tableView(_ tableView: NSTableView, shouldReorderColumn column: Int, toColumn newColumn: Int) -> Bool {
        // AppKit first asks with -1 whether dragging may begin, before proposing a destination.
        tableView.tableColumns[column].identifier != rowHeaderID && (newColumn == -1 || newColumn > 0)
    }

    func tableViewColumnDidMove(_ notification: Notification) {
        guard !applyingColumns else { return }
        model.reorderColumns(displayedColumns)
        requestHighlights()
    }

    func tableView(_ tableView: NSTableView, sizeToFitWidthOfColumn column: Int) -> CGFloat {
        if let source = columnIndex(table.tableColumns[column]) {
            // Start after AppKit finishes its divider resize notification.
            DispatchQueue.main.async { [weak self] in self?.fitColumns([UInt32(source)], automatic: false) }
        }
        return table.tableColumns[column].width
    }

    func tableViewColumnDidResize(_ notification: Notification) {
        guard !applyingWidths,
            let column = notification.userInfo?["NSTableColumn"] as? NSTableColumn,
            let source = columnIndex(column)
        else { return }
        widths.manual(UInt32(source))
    }

    private func fitVisibleColumns() {
        guard builtColumns, !fittingAutomatically, view.window != nil, displayedView != nil else { return }
        let columns = table.columnIndexes(in: table.visibleRect).compactMap { index -> UInt32? in
            let col = table.tableColumns[index]
            guard !col.isHidden, let source = columnIndex(col), widths.needsAutomatic(UInt32(source)) else {
                return nil
            }
            return UInt32(source)
        }
        if columns.isEmpty {
            if firstWidthsPending {
                table.alphaValue = 1
                firstWidthsPending = false
            }
            return
        }
        fitColumns(columns, automatic: true)
    }

    private func fitColumns(_ columns: [UInt32], automatic: Bool) {
        guard let view = displayedView, !columns.isEmpty else { return }
        if automatic { fittingAutomatically = true } else { fitTask?.cancel() }
        let tickets = columns.map { widths.begin($0) }
        let rows = table.rows(in: table.visibleRect)
        let start = rows.location == NSNotFound ? 0 : UInt64(rows.location)
        let inputs = columns.map { column in
            let info = model.info.columns[Int(column)]
            let key = RowCache.Key(start: Int(start) / RowCache.blockSize * RowCache.blockSize, column: column)
            let sample = cache.block(for: key).map { Array($0.cells.prefix(32)) }
            return ColumnWidthMeasurement.Input(
                column: column, name: info.name,
                type: info.displayType, text: info.valueCategory == .text, sample: sample)
        }
        let fontSize = textSize
        let worker = widthWorker
        let cancellation = ColumnWidthMeasurement.Request()
        let task = Task { [weak self] in
            var continueFitting = true
            defer {
                if let self {
                    if automatic { self.fittingAutomatically = false }
                    self.table.alphaValue = 1
                    self.firstWidthsPending = false
                    if automatic, continueFitting { self.fitVisibleColumns() }
                }
            }
            do {
                let result: [UInt32: CGFloat] = try await withTaskCancellationHandler {
                    try Task.checkCancellation()
                    return try await withCheckedThrowingContinuation { continuation in
                        worker.async {
                            do {
                                continuation.resume(
                                    returning: try ColumnWidthMeasurement.measure(
                                        view: view, columns: inputs, start: start, fontSize: fontSize,
                                        maximum: automatic ? 420 : 600, cancellation: cancellation))
                            } catch { continuation.resume(throwing: error) }
                        }
                    }
                } onCancel: {
                    cancellation.cancel()
                }
                try Task.checkCancellation()
                guard let self else { return }
                guard self.displayedView === view, self.textSize == fontSize else {
                    for ticket in tickets { self.widths.abandon(ticket) }
                    return
                }
                self.applyingWidths = true
                defer { self.applyingWidths = false }
                for ticket in tickets where self.widths.accept(ticket) {
                    if let column = self.table.tableColumn(withIdentifier: .init("c\(ticket.column)")),
                        let width = result[ticket.column]
                    {
                        column.width = width
                    }
                }
            } catch {
                guard let self else { return }
                for ticket in tickets { self.widths.abandon(ticket) }
                if Task.isCancelled, self.displayedView === view { continueFitting = false }
                if !Task.isCancelled, !(error is CancellationError), self.displayedView === view {
                    continueFitting = false
                    self.model.reportError(error, prefix: "Could not fit columns: ")
                }
            }
        }
        if automatic { automaticFitTask = task } else { fitTask = task }
    }

    func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
        guard let column = columnIndex(tableColumn) else { return }
        let multiple = (table.headerView as? FilterHeaderView)?.sortingMultiple ?? false
        DispatchQueue.main.async { [weak self] in
            self?.model.selectedColumn = column
            self?.model.cycleSort(column: UInt32(column), multiple: multiple)
        }
    }

    private func columnMenu(at index: Int) -> NSMenu? {
        guard index >= 0, index < table.tableColumns.count, let column = columnIndex(table.tableColumns[index]) else {
            return nil
        }
        let menu = NSMenu()
        for (title, action) in [
            ("Analyze Column", #selector(analyzeMenuColumn(_:))),
            ("Filter Column…", #selector(filterMenuColumn(_:))),
            ("Sort Ascending", #selector(sortMenuAscending(_:))),
            ("Sort Descending", #selector(sortMenuDescending(_:))),
            ("Fit Column Width", #selector(fitMenuColumn(_:))),
            ("Fit All Columns", #selector(fitMenuAll(_:))),
            ("Customize Columns…", #selector(customizeMenuColumns(_:))),
            ("Show All Columns", #selector(showMenuColumns(_:))),
            ("Hide Column", #selector(hideMenuColumn(_:))),
            ("Cancel Column Sort", #selector(clearMenuColumnSort(_:))),
            ("Clear All Sorting", #selector(clearMenuSort(_:))),
        ] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = UInt32(column)
            menu.addItem(item)
        }
        return menu
    }

    @objc private func analyzeMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.inspectColumn(Int(column)) }
    }
    @objc private func sortMenuAscending(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.setSortDirection(column: column, descending: false) }
    }
    @objc private func sortMenuDescending(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.setSortDirection(column: column, descending: true) }
    }
    @objc private func fitMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { fitColumns([column], automatic: false) }
    }
    @objc private func fitMenuAll(_ item: NSMenuItem) { fitColumns(displayedColumns, automatic: false) }
    @objc private func customizeMenuColumns(_ item: NSMenuItem) { model.showsColumns = true }
    @objc private func showMenuColumns(_ item: NSMenuItem) { model.showAllColumns() }

    @objc private func filterMenuColumn(_ item: NSMenuItem) {
        guard let column = item.representedObject as? UInt32 else { return }
        showFilter(at: table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)")))
    }
    @objc private func hideMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.hideColumn(column) }
    }
    @objc private func clearMenuColumnSort(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.removeSort(column: column) }
    }
    @objc private func clearMenuSort(_ item: NSMenuItem) { model.clearSort() }

    @objc private func tableClicked(_ sender: Any?) { selectColumn(at: table.clickedColumn) }
    private func selectColumn(at index: Int) {
        guard index >= 0, index < table.tableColumns.count, let column = columnIndex(table.tableColumns[index]) else {
            return
        }
        model.selectedColumn = column
    }

    func scrollToColumn(_ column: Int, focus: Bool = false) {
        let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
        guard index >= 0 else { return }
        table.scrollColumnToVisible(index)
        if focus { table.window?.makeFirstResponder(table) }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !reloading else { return }
        gutter.needsDisplay = true
        let row = table.selectedRow
        model.selectedDisplayRow = row >= 0 ? row : nil
        if row >= 0, let block = block(for: row, column: nil), row - Int(block.start) < block.sourceRows.count {
            model.selectedSourceRow = block.sourceRows[row - Int(block.start)]
        } else {
            model.selectedSourceRow = nil
        }
    }

    func navigateToMatch(_ match: CellMatch) {
        let row = Int(match.displayRow)
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        table.scrollRowToVisible(row)
        scrollToColumn(Int(match.column))
        requestHighlights()
    }

    func showSelectedColumnFilter() {
        guard let column = model.selectedColumn,
            let index = table.tableColumns.firstIndex(where: { $0.identifier.rawValue == "c\(column)" })
        else { return }
        showFilter(at: index)
    }

    func showColumnFilter(_ column: UInt32) {
        guard model.appliedSpec.columns.contains(where: { $0.column == column }) else { return }
        scrollToColumn(Int(column))
        showFilter(at: table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)")))
    }

    var displayedColumns: [UInt32] {
        table.tableColumns.filter { !$0.isHidden }.compactMap(columnIndex).map(UInt32.init)
    }

    private func showFilter(at index: Int) {
        guard let header = table.headerView,
            index >= 0, index < table.tableColumns.count, let col = columnIndex(table.tableColumns[index])
        else { return }
        closePopover()
        let column = UInt32(col)
        let content = ColumnFilterView(
            viewer: model.viewer, column: column, info: model.info.columns[col],
            current: model.spec.columns.first { $0.column == column }?.filter
        ) { [weak self] filters in
            self?.closePopover()
            self?.model.setFilter(filters, column: column)
        }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(rootView: content)
        popover.show(relativeTo: header.headerRect(ofColumn: index), of: header, preferredEdge: .maxY)
        self.popover = popover
    }

    func closePopover() {
        popover?.close()
        popover = nil
    }

    @objc func copy(_ sender: Any?) {
        guard !table.selectedRowIndexes.isEmpty else { return }
        let columns = displayedColumns
        model.copying.copy(
            view: model.view,
            rows: Array(table.selectedRowIndexes.rangeView), columns: columns,
            names: columns.map { model.info.columns[Int($0)].name })
    }
}
