import AppKit
import SwiftUI

/// SwiftUI updates this bridge frequently. The controller reloads rows only
/// when Rust publishes a new view version, preserving the live NSTableView.
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
    private var failedVersion: UInt64?
    private var initialFitsRemaining = 32
    private var initiallyFitted: Set<Int> = []
    private var popover: NSPopover?
    private var builtColumns = false
    private var version: UInt64?
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
        header.onColumnClick = { [weak self] in self?.selectColumn(at: $0) }
        table.headerView = header
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
        let findChanged = findRevision != model.find.revision || highlightRevision != model.find.highlightRevision
        findRevision = model.find.revision
        highlightRevision = model.find.highlightRevision
        let effectiveHeight = max(rowHeight, fontSize + 8)
        let presentationChanged =
            textSize != fontSize || table.rowHeight != effectiveHeight || self.rowNumberBase != rowNumberBase
        if presentationChanged {
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
        if version != model.current.version {
            version = model.current.version
            stopLoading()
            cache.clear()
            failedVersion = nil
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
        for column in table.tableColumns {
            guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
            header.textSize = textSize
            header.filtered = model.appliedFilterColumns.contains(UInt32(i))
            let sort = model.appliedSpec.sort.first { $0.column == UInt32(i) }
            header.sorted = sort != nil
            header.sortAscending = !(sort?.descending ?? false)
        }
        table.headerView?.needsDisplay = true
        requestHighlights()
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    @objc private func visibleRectChanged() { requestHighlights() }

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
                "\(column.name): \(column.technicalType)\nClick to sort; use the funnel to filter.\nDrag to reorder; double-click a divider to fit the column."
            c.width = fittedWidth(column: i, texts: [], maxWidth: 320)
            c.minWidth = 50
            c.sortDescriptorPrototype = NSSortDescriptor(key: "c\(i)", ascending: true)
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

    private func fittedWidth(column: Int, texts: [String?], maxWidth: CGFloat) -> CGFloat {
        let info = model.info
        func width(_ text: String, font: NSFont) -> CGFloat {
            ceil((text as NSString).size(withAttributes: [.font: font]).width)
        }
        let c = info.columns[column]
        var result =
            max(
                width(c.name, font: .systemFont(ofSize: textSize, weight: .semibold)),
                width(c.displayType, font: .systemFont(ofSize: textSize - 2))) + 50
        for text in texts.prefix(200) {
            result = max(
                result, width(String((text ?? "null").prefix(200)), font: fonts.font(for: model.kinds[column])) + 20)
        }
        return min(max(result, 60), maxWidth)
    }

    private func columnIndex(_ column: NSTableColumn) -> Int? {
        let id = column.identifier.rawValue
        return id.hasPrefix("c") ? Int(id.dropFirst()) : nil
    }

    private func block(for row: Int, column: UInt32?) -> RowBlock? {
        let key = RowCache.Key(start: row / RowCache.blockSize * RowCache.blockSize, column: column)
        if let block = cache.block(for: key) { return block }
        guard failedVersion != version, loading != key, !pending.contains(key) else { return nil }
        // Bound queued work as well as cached data. Completion revisits visible
        // placeholders, so a full queue never permanently drops a visible cell.
        if pending.count < 128 { pending.append(key) }
        loadNext()
        return nil
    }

    private func loadNext() {
        guard loading == nil, !pending.isEmpty, let version else { return }
        let doc = model.doc
        let key = pending.removeFirst()
        loading = key
        let reader = reader
        loadTask = Task { [weak self] in
            do {
                let block = try await reader.read(
                    doc, version: version, start: UInt64(key.start),
                    count: UInt32(RowCache.blockSize), columns: key.column.map { [$0] } ?? [])
                guard !Task.isCancelled, let self, self.version == version else { return }
                self.cache.insert(block, for: key)
                self.display(block, column: key.column)
                self.loading = nil
                self.fillVisiblePlaceholders()
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.version == version else { return }
                self.loading = nil
                self.pending.removeAll()
                self.failedVersion = version
                if case GewuError.StaleVersion = error { return }
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
            // At most 32 * 32 cells contribute to automatic width sampling.
            if initialFitsRemaining > 0, !initiallyFitted.contains(Int(column)) {
                initiallyFitted.insert(Int(column))
                initialFitsRemaining -= 1
                table.tableColumns[index].width = fittedWidth(
                    column: Int(column), texts: Array(block.cells.prefix(32)), maxWidth: 320)
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
        pending.removeAll()
        loading = nil
    }

    func numberOfRows(in tableView: NSTableView) -> Int { Int(model.current.rowCount) }

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
        guard let field = cell.textField else { return cell }
        let kind = model.kinds[col]
        cell.wholeCellMatch = false
        cell.isLoading = true
        field.stringValue = "…"
        field.font = fonts.font(for: kind)
        field.alignment = kind.alignment
        field.textColor = .tertiaryLabelColor
        if let block = block(for: row, column: UInt32(col)) { render(cell, column: col, row: row, block: block) }
        return cell
    }

    private func render(_ cell: MatchCellView, column: Int, row: Int, block: RowBlock) {
        guard let field = cell.textField, row >= Int(block.start), row < Int(block.start) + Int(block.nRows) else {
            return
        }
        let kind = model.kinds[column]
        let index = row - Int(block.start)
        let hit = model.find.isMatch(row: row, column: column)
        cell.isLoading = false
        cell.wholeCellMatch = hit && kind != .text
        if let text = block.cells[index] {
            if hit, kind == .text, let query = model.find.activeQuery {
                field.attributedStringValue = highlighted(text, query: query, font: fonts.font(for: kind))
            } else {
                field.stringValue = text
                field.textColor = hit ? .black : .labelColor
            }
            field.font = fonts.font(for: kind)
        } else {
            field.stringValue = "null"
            field.font = fonts.null
            field.textColor = .tertiaryLabelColor
        }
        field.alignment = kind.alignment
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
        table.tableColumns[column].identifier != rowHeaderID && newColumn > 0
    }

    func tableViewColumnDidMove(_ notification: Notification) {
        model.findColumnsChanged()
        requestHighlights()
    }

    func tableView(_ tableView: NSTableView, sizeToFitWidthOfColumn column: Int) -> CGFloat {
        guard let i = columnIndex(table.tableColumns[column]) else { return table.tableColumns[column].width }
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return table.tableColumns[column].width }
        let doc = model.doc
        let version = model.current.version
        let start = visible.location
        let count = min(200, visible.length)
        fitTask?.cancel()
        let reader = reader
        fitTask = Task { [weak self] in
            do {
                let block = try await reader.read(
                    doc, version: version, start: UInt64(start),
                    count: UInt32(count), columns: [UInt32(i)])
                guard !Task.isCancelled, let self, self.version == version,
                    let target = self.table.tableColumn(withIdentifier: NSUserInterfaceItemIdentifier("c\(i)"))
                else { return }
                self.initiallyFitted.insert(i)
                target.width = self.fittedWidth(column: i, texts: block.cells, maxWidth: 600)
            } catch {
                guard !Task.isCancelled, let self, self.version == version else { return }
                self.model.reportError(error, prefix: "Could not fit column: ")
            }
        }
        return table.tableColumns[column].width
    }

    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        guard let first = table.sortDescriptors.first, let key = first.key,
            let column = UInt32(key.dropFirst())
        else {
            model.setSort([])
            return
        }
        if table.sortDescriptors.count > 1 {
            table.sortDescriptors = [first]
            return
        }
        model.setSort([SortKey(column: column, descending: !first.ascending)])
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

    var displayedColumns: [UInt32] { table.tableColumns.compactMap(columnIndex).map(UInt32.init) }

    private func showFilter(at index: Int) {
        guard let header = table.headerView,
            index >= 0, index < table.tableColumns.count, let col = columnIndex(table.tableColumns[index])
        else { return }
        closePopover()
        let column = UInt32(col)
        let content = ColumnFilterView(
            doc: model.doc, column: column, info: model.info.columns[col],
            querySpec: model.spec, current: model.columnFilters[column] ?? []
        ) { [weak self] filters in
            self?.closePopover()
            self?.model.setFilters(filters, column: column)
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
            document: model.doc, version: model.current.version,
            rows: Array(table.selectedRowIndexes.rangeView), columns: columns,
            names: columns.map { model.info.columns[Int($0)].name })
    }
}
