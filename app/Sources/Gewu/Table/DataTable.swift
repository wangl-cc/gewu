import AppKit
import SwiftUI

/// The native table consumes its own presentation, independent of window panels
/// and pending queries. Find updates decorate existing cells.
struct DataTable: NSViewControllerRepresentable {
    let model: DocumentModel
    @ObservedObject private var presentation: TablePresentation
    // Observed only so SwiftUI calls `update` when Find state or highlights change.
    @ObservedObject private var find: FindModel
    var showsRowNumbers: Bool
    @AppStorage("TableFontSize") private var fontSize = 12.0
    @AppStorage("TableRowHeight") private var rowHeight = 28.0
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    init(model: DocumentModel, showsRowNumbers: Bool) {
        self.model = model
        _presentation = ObservedObject(wrappedValue: model.tablePresentation)
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
    private let table = ContextTableView()
    private let scroll = NSScrollView()
    private lazy var gutter = RowGutterView(table: table)
    private let rows: TableRowLoader
    private var fitTask: Task<Void, Never>?
    private var findRevision: UInt64?
    private var highlightRevision: UInt64?
    private var currentFindMatch: CellMatch?
    private var widths = ColumnWidths()
    private var automaticFitTask: Task<Void, Never>?
    private var fittingAutomatically = false
    private var applyingWidths = false
    private var firstWidthsPending = true
    private let widthWorker = DispatchQueue(label: "gewu.table.widths", qos: .userInitiated)
    private var popover: NSPopover?
    private var builtColumns = false
    private var applyingColumns = false
    private var displayedInput: TablePresentation.Input?
    private var displayedView: GewuView? { displayedInput?.rows }
    private var viewportRows: NSRange?
    private var viewportColumns: [UInt32] = []
    private var showsNumbers: Bool?
    private var reloading = false
    private var textSize: CGFloat = 12
    private var fonts = CellFonts()
    private var rowNumberBase = 0
    private let rowHeaderID = NSUserInterfaceItemIdentifier("rowHeader")
    private let rowViewID = NSUserInterfaceItemIdentifier("stripedRow")

    init(model: DocumentModel, loader: TableRowLoader? = nil) {
        self.model = model
        rows = loader ?? TableRowLoader()
        super.init(nibName: nil, bundle: nil)
        rows.onLoad = { [weak self] view, block, columns in
            guard let self, self.displayedView === view, self.model.applied.rowView === view else { return }
            self.display(block, columns: columns)
        }
        rows.onError = { [weak self] view, error in
            guard let self, self.displayedView === view, self.model.applied.rowView === view else { return }
            self.model.reportError(error, prefix: "Could not load rows: ")
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// Predict the visible region after row-count and column-layout changes.
    /// Include one bordering row for fractional scrolling and AppKit's clipping.
    private func viewport(for view: GewuView, spec: QuerySpec) -> TableRowLoader.Viewport {
        let count = Int(clamping: view.rowCount())
        let clip = scroll.contentView.bounds
        let stride = table.rowHeight + table.intercellSpacing.height
        let y = max(0, min(clip.minY, CGFloat(count) * stride - clip.height))
        let first = max(0, Int(floor(y / stride)) - 1)
        let last = min(count, Int(ceil((y + clip.height) / stride)) + 1)
        let gutterWidth = showsNumbers == true ? gutter.fittedWidth : 0
        let widths = spec.columns.map { col in
            table.tableColumn(withIdentifier: .init("c\(col.column)"))?.width ?? 140
        }
        let totalWidth = widths.reduce(gutterWidth, +)
        let x = max(0, min(clip.minX, totalWidth - clip.width))
        var edge = gutterWidth
        var columns: [UInt32] = []
        for (col, width) in zip(spec.columns, widths) {
            if edge + width > x && edge < x + clip.width { columns.append(col.column) }
            edge += width
        }
        return .init(rows: min(first, count)..<max(min(first, count), last), columns: columns)
    }

    func loadViewport(in view: GewuView, spec: QuerySpec) async throws -> TableRowLoader.Snapshot {
        while true {
            let snapshot = try await rows.snapshot(of: view, viewport: viewport(for: view, spec: spec))
            try Task.checkCancellation()
            // The user can scroll, resize or change column widths during the read.
            if viewport(for: view, spec: spec) == snapshot.viewport { return snapshot }
        }
    }

    func commit(_ snapshot: TableRowLoader.Snapshot) {
        loadViewIfNeeded()
        closePopover()
        stopFitting()
        let oldCount = displayedView?.rowCount()
        reloading = true
        rows.install(snapshot)
        updateColumns(model.tablePresentation.input)
        table.deselectAll(nil)
        if oldCount != snapshot.view.rowCount() { table.noteNumberOfRowsChanged() }
        table.layoutSubtreeIfNeeded()

        // Visible cells receive the prepared values directly. Retained row views
        // outside this viewport must also lose their old-order content.
        let visible = table.rows(in: table.visibleRect)
        let visibleColumns = Set(
            table.columnIndexes(in: table.visibleRect).compactMap { columnIndex(table.tableColumns[$0]) })
        let preparedColumns = Set(snapshot.viewport.columns)
        table.enumerateAvailableRowViews { rowView, row in
            for case let cell as MatchCellView in rowView.subviews {
                guard let id = cell.identifier, let column = Int(id.rawValue.dropFirst()) else { continue }
                if NSLocationInRange(row, visible), visibleColumns.contains(column),
                    snapshot.viewport.rows.contains(row), preparedColumns.contains(UInt32(column))
                {
                    continue
                }
                self.bind(cell, column: column, row: row)
            }
        }
        for page in snapshot.pages { display(page.block, columns: page.columns) }
        gutter.needsDisplay = true
        reloading = false
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    override func loadView() {
        let root = NSView()
        let header = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: 32))
        header.onFilterClick = { [weak self] in self?.showFilter(at: $0) }
        header.columnMenu = { [weak self] in self?.columnMenu(at: $0) }

        table.headerView = header
        table.alphaValue = 0
        table.target = self
        table.action = #selector(tableClicked(_:))
        table.cellMenu = { [weak self] row, column in self?.cellMenu(row: row, index: column) }
        table.doubleAction = #selector(inspectClickedCell(_:))
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
        buildColumns(model.info)
        builtColumns = true
        rows.reset(to: model.tablePresentation.input.rows)
        updateColumns(model.tablePresentation.input)
        table.noteNumberOfRowsChanged()
    }

    /// SwiftUI updates presentation; replacing row data goes through commit.
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
            if textSize != fontSize { widths.fontChanged() }
            textSize = fontSize
            fonts = CellFonts(size: fontSize)
            self.rowNumberBase = rowNumberBase
            table.rowHeight = effectiveHeight
            table.headerView?.frame.size.height = max(32, ceil(fontSize * 2.6))
            scroll.tile()
            gutter.font = fonts.rowNumber
        }
        if showsNumbers != showsRowNumbers {
            showsNumbers = showsRowNumbers
            gutter.setShown(showsRowNumbers)
            table.tableColumn(withIdentifier: rowHeaderID)?.isHidden = !showsRowNumbers
        }
        if presentationChanged {
            fitGutter(info)
            gutter.needsDisplay = true
        }
        updateColumns(model.tablePresentation.input, restyle: presentationChanged)
        if presentationChanged || findChanged {
            refreshVisibleCells(restyle: presentationChanged)
        }
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    private func updateColumns(_ input: TablePresentation.Input, restyle: Bool = false) {
        let previous = displayedInput
        displayedInput = input
        let visible = input.spec.columns.map(\.column)
        if previous?.spec.columns.map(\.column) != visible {
            applyingColumns = true
            let visibleSet = Set(visible)
            for column in table.tableColumns {
                if let i = columnIndex(column) { column.isHidden = !visibleSet.contains(UInt32(i)) }
            }
            for (offset, column) in visible.enumerated() {
                let from = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
                if from != offset + 1 { table.moveColumn(from, toColumn: offset + 1) }
            }
            applyingColumns = false
        }
        if previous != input || restyle {
            let columns = Dictionary(uniqueKeysWithValues: input.spec.columns.map { ($0.column, $0) })
            let oldColumns = Dictionary(uniqueKeysWithValues: (previous?.spec.columns ?? []).map { ($0.column, $0) })
            let multipleSortKeys = input.spec.columns.filter { $0.sort != nil }.count > 1
            let hadMultipleSortKeys = (previous?.spec.columns.filter { $0.sort != nil }.count ?? 0) > 1
            for column in table.tableColumns {
                guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
                if previous != nil, !restyle,
                    oldColumns[UInt32(i)] == columns[UInt32(i)], multipleSortKeys == hadMultipleSortKeys,
                    (previous?.selectedColumn == i) == (input.selectedColumn == i),
                    (previous?.inspectedColumn == i) == (input.inspectedColumn == i)
                {
                    continue
                }
                header.textSize = textSize
                let filter = columns[UInt32(i)]?.filter
                header.filtered = filter != nil
                header.subtitle = model.info.columns[i].displayType
                column.headerToolTip =
                    "\(model.info.columns[i].name): \(model.info.columns[i].technicalType)"
                    + (filter.map { "\n\($0.detailSummary)" } ?? "")
                    + "\nClick to cycle sorting; Shift-click for multiple keys. Use the funnel to filter."
                    + "\nDrag to reorder; double-click a divider to fit the column."
                header.inspected = input.inspectedColumn == i
                header.active = input.selectedColumn == i
                let sort = columns[UInt32(i)]?.sort
                header.sorted = sort != nil
                header.sortPriority = multipleSortKeys ? (sort.map { Int($0.priority) + 1 } ?? 0) : 0
                header.sortAscending = !(sort?.descending ?? false)
            }
            table.headerView?.needsDisplay = true
        }
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
        updateViewport()
        fitVisibleColumns()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        updateViewport()
        fitVisibleColumns()
    }

    @objc private func visibleRectChanged() {
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    private func updateViewport() {
        guard builtColumns, !reloading, !applyingColumns else { return }
        let visibleRows = table.rows(in: table.visibleRect)
        guard visibleRows.location != NSNotFound else {
            rows.updateViewport(rows: 0..<0, columns: [])
            return
        }
        let columns = table.columnIndexes(in: table.visibleRect).compactMap { index -> UInt32? in
            let column = table.tableColumns[index]
            guard !column.isHidden, let source = columnIndex(column) else { return nil }
            return UInt32(source)
        }
        if viewportRows != visibleRows || viewportColumns != columns {
            refreshVisibleCells(restyle: true, enteringViewport: true)
            viewportRows = visibleRows
            viewportColumns = columns
        }
        rows.updateViewport(
            rows: visibleRows.location..<(visibleRows.location + visibleRows.length), columns: columns)
    }

    private func requestHighlights() {
        let rows = table.rows(in: table.visibleRect)
        guard rows.location != NSNotFound, rows.length > 0 else { return }
        model.find.requestHighlight(start: UInt64(rows.location), count: UInt32(rows.length))
    }

    private func buildColumns(_ info: DatasetInfo) {
        gutter.label = { [weak self] row in
            guard let self, let block = self.rows.block(for: row, column: nil) else { return nil }
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

    private func fitGutter(_ info: DatasetInfo) {
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

    private func display(_ block: RowBlock, columns: [UInt32]) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        let start = max(Int(block.start), visible.location)
        let end = min(Int(block.start) + Int(block.nRows), visible.location + visible.length)
        guard start < end else { return }
        let visibleColumns = table.columnIndexes(in: table.visibleRect)
        for (offset, column) in columns.enumerated() {
            let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
            guard visibleColumns.contains(index) else { continue }
            for row in start..<end {
                if let cell = table.view(atColumn: index, row: row, makeIfNecessary: true) as? MatchCellView {
                    render(cell, column: Int(column), row: row, block: block, offset: offset)
                }
            }
        }
        gutter.needsDisplay = true
        if let row = model.selectedDisplayRow, row >= Int(block.start), row < Int(block.start) + Int(block.nRows) {
            let source = block.sourceRows[row - Int(block.start)]
            if model.selectedSourceRow != source { model.selectedSourceRow = source }
        }
    }

    /// Decoration and style changes operate on the content already in the cell,
    /// including large values intentionally not retained by the row cache.
    private func refreshVisibleCells(restyle: Bool, enteringViewport: Bool = false) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        for index in table.columnIndexes(in: table.visibleRect) {
            guard let column = columnIndex(table.tableColumns[index]) else { continue }
            for row in visible.location..<(visible.location + visible.length) {
                if enteringViewport, let viewportRows, NSLocationInRange(row, viewportRows),
                    viewportColumns.contains(UInt32(column))
                {
                    continue
                }
                guard let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView else {
                    continue
                }
                if cell.isLoading, let block = rows.block(for: row, column: UInt32(column)) {
                    render(cell, column: column, row: row, block: block)
                } else {
                    let kind = model.kinds[column]
                    if restyle {
                        cell.updateStyle(
                            font: cell.muted && !cell.isLoading ? fonts.null : fonts.font(for: kind),
                            alignment: kind.alignment)
                    }
                    let hit = !cell.isLoading && model.find.isMatch(row: row, column: column)
                    let current = model.find.currentMatch
                    cell.updateFind(
                        query: hit && kind == .text ? model.find.activeQuery : nil,
                        wholeMatch: hit && kind != .text,
                        current: current?.displayRow == UInt64(row) && current?.column == UInt32(column))
                }
            }
        }
    }

    func stopLoading() {
        rows.stop()
        stopFitting()
    }

    private func stopFitting() {
        fitTask?.cancel()
        automaticFitTask?.cancel()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { Int(displayedView?.rowCount() ?? 0) }

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
        bind(cell, column: col, row: row)
        return cell
    }

    private func bind(_ cell: MatchCellView, column: Int, row: Int) {
        if let block = rows.block(for: row, column: UInt32(column)) {
            render(cell, column: column, row: row, block: block)
        } else {
            let kind = model.kinds[column]
            cell.isLoading = true
            cell.configure(text: "…", font: fonts.font(for: kind), alignment: kind.alignment, muted: true)
        }
    }

    private func render(_ cell: MatchCellView, column: Int, row: Int, block: RowBlock, offset: Int = 0) {
        guard row >= Int(block.start), row < Int(block.start) + Int(block.nRows) else {
            return
        }
        let kind = model.kinds[column]
        let index = (row - Int(block.start)) * Int(block.nCols) + offset
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

    private struct CellContent {
        let row: Int
        let column: Int
        let text: String?
    }

    private func cellContent(row: Int, index: Int) -> CellContent? {
        guard row >= 0, index >= 0, index < table.tableColumns.count,
            let column = columnIndex(table.tableColumns[index])
        else { return nil }
        if let block = rows.block(for: row, column: UInt32(column)),
            row >= Int(block.start), row < Int(block.start) + Int(block.nRows)
        {
            return CellContent(row: row, column: column, text: block.cells[row - Int(block.start)])
        }
        // Large values can be displayed without being retained by the row cache.
        guard let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView,
            !cell.isLoading, let field = cell.textField
        else { return nil }
        return CellContent(row: row, column: column, text: cell.muted ? nil : field.stringValue)
    }

    private func cellMenu(row: Int, index: Int) -> NSMenu? {
        guard let content = cellContent(row: row, index: index) else { return nil }
        selectColumn(at: index)
        if !table.selectedRowIndexes.contains(row) {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        let menu = NSMenu()
        for (title, action) in [
            ("Copy Cell", #selector(copyCell(_:))),
            ("Copy Selected Rows", #selector(copy(_:))),
            ("View Cell…", #selector(inspectCell(_:))),
        ] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = content
            menu.addItem(item)
        }
        menu.addItem(.separator())
        for (title, action) in [
            ("Filter Column…", #selector(filterMenuColumn(_:))),
            ("Clear Column Filter", #selector(clearMenuColumnFilter(_:))),
            ("Analyze Column", #selector(analyzeMenuColumn(_:))),
        ] {
            if action == #selector(clearMenuColumnFilter(_:)),
                !model.appliedFilterColumns.contains(UInt32(content.column))
            {
                continue
            }
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = UInt32(content.column)
            menu.addItem(item)
        }
        return menu
    }

    @objc private func copyCell(_ item: NSMenuItem) {
        guard let content = item.representedObject as? CellContent else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(content.text ?? "", forType: .string)
    }

    @objc private func inspectCell(_ item: NSMenuItem) {
        if let content = item.representedObject as? CellContent { showCell(content) }
    }

    @objc private func inspectClickedCell(_ sender: Any?) {
        if let content = cellContent(row: table.clickedRow, index: table.clickedColumn) { showCell(content) }
    }

    private func showCell(_ content: CellContent) {
        closePopover()
        let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(content.column)"))
        guard index >= 0 else { return }
        let panel = NSPopover()
        panel.behavior = .transient
        panel.contentViewController = NSHostingController(
            rootView: CellPreview(
                name: model.info.columns[content.column].name, text: content.text))
        panel.show(relativeTo: table.frameOfCell(atColumn: index, row: content.row), of: table, preferredEdge: .maxY)
        popover = panel
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
            let sample = self.rows.cachedBlock(for: key).map { Array($0.cells.prefix(32)) }
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
        let sort = model.spec.columns.first { $0.column == UInt32(column) }?.sort
        for (title, action) in [
            ("Analyze Column", #selector(analyzeMenuColumn(_:))),
            ("Filter Column…", #selector(filterMenuColumn(_:))),
            ("Clear Column Filter", #selector(clearMenuColumnFilter(_:))),
            ("", #selector(filterMenuColumn(_:))),
            ("Sort Ascending", #selector(sortMenuAscending(_:))),
            ("Sort Descending", #selector(sortMenuDescending(_:))),
            ("Clear Column Sort", #selector(clearMenuColumnSort(_:))),
            ("Clear All Sorting", #selector(clearMenuAllSorting(_:))),
            ("", #selector(filterMenuColumn(_:))),
            ("Fit Column Width", #selector(fitMenuColumn(_:))),
            ("Fit All Columns", #selector(fitMenuAll(_:))),
            ("", #selector(filterMenuColumn(_:))),
            ("Customize Columns…", #selector(customizeMenuColumns(_:))),
            ("Hide Column", #selector(hideMenuColumn(_:))),
        ] {
            if title.isEmpty {
                menu.addItem(.separator())
                continue
            }
            if action == #selector(clearMenuColumnSort(_:)), sort == nil { continue }
            if action == #selector(clearMenuAllSorting(_:)), model.sortedColumns.isEmpty { continue }
            if action == #selector(clearMenuColumnFilter(_:)),
                !model.appliedFilterColumns.contains(UInt32(column))
            {
                continue
            }
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = UInt32(column)
            if action == #selector(sortMenuAscending(_:)), sort?.descending == false { item.state = .on }
            if action == #selector(sortMenuDescending(_:)), sort?.descending == true { item.state = .on }
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

    @objc private func filterMenuColumn(_ item: NSMenuItem) {
        guard let column = item.representedObject as? UInt32 else { return }
        showFilter(at: table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)")))
    }
    @objc private func hideMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { hideColumn(column, in: model) }
    }
    @objc private func clearMenuColumnSort(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.removeSort(column: column) }
    }
    @objc private func clearMenuAllSorting(_ item: NSMenuItem) { model.clearSort() }
    @objc private func clearMenuColumnFilter(_ item: NSMenuItem) {
        if let column = item.representedObject as? UInt32 { model.setFilter(nil, column: column) }
    }

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

    func focusTable() { table.window?.makeFirstResponder(table) }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !reloading else { return }
        gutter.needsDisplay = true
        let row = table.selectedRow
        model.selectedDisplayRow = row >= 0 ? row : nil
        if row >= 0, let block = rows.block(for: row, column: nil), row - Int(block.start) < block.sourceRows.count {
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

private final class ContextTableView: NSTableView {
    var cellMenu: ((Int, Int) -> NSMenu?)?

    override func menu(for event: NSEvent) -> NSMenu? {
        let point = convert(event.locationInWindow, from: nil)
        return cellMenu?(row(at: point), column(at: point))
    }
}

struct CellPreview: View {
    let name: String
    let text: String?

    private var textHeight: CGFloat {
        let bounds = ((text ?? "null") as NSString).boundingRect(
            with: NSSize(width: 400, height: CGFloat.greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading],
            attributes: [.font: NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)])
        return min(260, max(24, ceil(bounds.height) + 4))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(name).font(.headline).lineLimit(1)
                Spacer()
                Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text ?? "", forType: .string)
                }.buttonStyle(.borderless)
            }
            ScrollView {
                Text(text ?? "null").textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .font(.system(.body, design: .monospaced))
            }.frame(height: textHeight)
        }.padding(20).frame(width: 440)
    }
}
