import AppKit
import SwiftUI

/// SwiftUI updates this bridge frequently. The controller reloads rows only
/// when Rust publishes a new view version, preserving the live NSTableView.
struct DataTable: NSViewControllerRepresentable {
    @ObservedObject var model: DocumentModel
    var showsRowNumbers: Bool

    func makeNSViewController(context: Context) -> DataTableController {
        let controller = DataTableController(model: model)
        model.table = controller
        return controller
    }

    func updateNSViewController(_ controller: DataTableController, context: Context) {
        controller.update(showsRowNumbers: showsRowNumbers)
    }

    static func dismantleNSViewController(_ controller: DataTableController, coordinator: ()) {
        controller.closePopover()
    }
}

@MainActor
final class DataTableController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let model: DocumentModel
    private let table = NSTableView()
    private let scroll = NSScrollView()
    private lazy var gutter = RowGutterView(table: table)
    private let cache = RowCache(blockSize: 256)
    private var popover: NSPopover?
    private var builtColumns = false
    private var version: UInt64?
    private var showsNumbers: Bool?
    private var reloading = false
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
        header.onColumnClick = { [weak self] in self?.inspectColumn(at: $0) }
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

    func update(showsRowNumbers: Bool) {
        loadViewIfNeeded()
        guard let info = model.info else { return }
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
            cache.clear()
            reloading = true
            table.deselectAll(nil)
            table.reloadData()
            reloading = false
            gutter.needsDisplay = true
        }
        for column in table.tableColumns {
            guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
            header.filtered = model.columnFilters[UInt32(i)] != nil
            let sort = model.sort.first { $0.column == UInt32(i) }
            header.sorted = sort != nil
            header.sortAscending = !(sort?.descending ?? false)
        }
        table.headerView?.needsDisplay = true
    }

    private func buildColumns(_ info: FileInfo) {
        gutter.fit(largest: max(info.rowCount, 1) - 1)
        gutter.toolTip = "Row in file (0-based)"
        gutter.label = { [weak self] row in
            guard let block = self?.block(for: row) else { return nil }
            let r = row - Int(block.start)
            return r >= 0 && r < Int(block.nRows) ? String(block.sourceRows[r]) : nil
        }
        let rowHeader = NSTableColumn(identifier: rowHeaderID)
        rowHeader.headerCell = ColumnHeaderCell(textCell: "")
        rowHeader.width = gutter.fittedWidth
        rowHeader.minWidth = gutter.fittedWidth
        rowHeader.maxWidth = gutter.fittedWidth
        rowHeader.resizingMask = []
        table.addTableColumn(rowHeader)
        for (i, column) in info.columns.enumerated() {
            let c = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(i)"))
            let header = ColumnHeaderCell(textCell: column.name)
            header.subtitle = shortTypeName(column.typeName)
            header.showsFunnel = true
            c.headerCell = header
            c.headerToolTip = "\(column.name): \(column.typeName)"
            c.width = fittedWidth(column: i, rows: 0..<min(200, Int(model.current.rowCount)), maxWidth: 320)
            c.minWidth = 50
            c.sortDescriptorPrototype = NSSortDescriptor(key: "c\(i)", ascending: true)
            table.addTableColumn(c)
        }
    }

    private func fittedWidth(column: Int, rows: Range<Int>, maxWidth: CGFloat) -> CGFloat {
        guard let info = model.info else { return 150 }
        func width(_ text: String, font: NSFont) -> CGFloat {
            ceil((text as NSString).size(withAttributes: [.font: font]).width)
        }
        let c = info.columns[column]
        var result = max(width(c.name, font: .systemFont(ofSize: 12, weight: .semibold)),
                         width(shortTypeName(c.typeName), font: .systemFont(ofSize: 10))) + 50
        let kind = model.kinds[column]
        for row in rows {
            guard let block = block(for: row) else { break }
            let r = row - Int(block.start)
            guard r >= 0, r < Int(block.nRows) else { continue }
            let text = block.cells[r * Int(block.nCols) + column] ?? "null"
            result = max(result, width(String(text.prefix(200)), font: kind.font) + 20)
        }
        return min(max(result, 60), maxWidth)
    }

    private func columnIndex(_ column: NSTableColumn) -> Int? {
        let id = column.identifier.rawValue
        return id.hasPrefix("c") ? Int(id.dropFirst()) : nil
    }

    private func block(for row: Int) -> RowBlock? {
        guard let doc = model.doc, let info = model.info else { return nil }
        return cache.block(for: row, version: model.current.version) { start, count in
            try? doc.rows(version: model.current.version, start: UInt64(start), count: UInt32(count),
                          columns: Array(0..<UInt32(info.columns.count)))
        }
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
        let cell = (table.makeView(withIdentifier: tableColumn.identifier, owner: self) as? MatchCellView)
            ?? makeCell(identifier: tableColumn.identifier)
        guard let field = cell.textField else { return cell }
        let kind = model.kinds[col]
        cell.wholeCellMatch = false
        guard let block = block(for: row),
              row >= Int(block.start), row - Int(block.start) < Int(block.nRows) else {
            field.stringValue = "…"
            field.font = kind.font
            field.alignment = kind.alignment
            field.textColor = .tertiaryLabelColor
            return cell
        }
        let r = row - Int(block.start)
        let index = r * Int(block.nCols) + col
        let hit = index < block.matched.count && block.matched[index]
        cell.wholeCellMatch = hit && kind != .text
        if let text = block.cells[index] {
            if hit, kind == .text, let query = model.appliedState.search {
                field.attributedStringValue = highlighted(text, query: query, font: kind.font)
            } else {
                field.stringValue = text
                field.textColor = hit ? .black : .labelColor
            }
            field.font = kind.font
        } else {
            field.stringValue = "null"
            field.font = CellFonts.null
            field.textColor = .tertiaryLabelColor
        }
        field.alignment = kind.alignment
        return cell
    }

    private func makeCell(identifier: NSUserInterfaceItemIdentifier) -> MatchCellView {
        let cell = MatchCellView()
        cell.identifier = identifier
        let field = NSTextField(labelWithString: "")
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

    func tableView(_ tableView: NSTableView, sizeToFitWidthOfColumn column: Int) -> CGFloat {
        guard let i = columnIndex(table.tableColumns[column]) else { return table.tableColumns[column].width }
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return table.tableColumns[column].width }
        return fittedWidth(column: i, rows: visible.location..<(visible.location + visible.length), maxWidth: 600)
    }

    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        guard let first = table.sortDescriptors.first, let key = first.key,
              let column = UInt32(key.dropFirst()) else { model.setSort([]); return }
        if table.sortDescriptors.count > 1 { table.sortDescriptors = [first]; return }
        model.setSort([SortKey(column: column, descending: !first.ascending)])
    }

    @objc private func tableClicked(_ sender: Any?) { inspectColumn(at: table.clickedColumn) }
    private func inspectColumn(at index: Int) {
        guard index >= 0, index < table.tableColumns.count, let column = columnIndex(table.tableColumns[index]) else { return }
        model.selectedColumn = column
    }

    func scrollToColumn(_ column: Int) {
        let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
        if index >= 0 { table.scrollColumnToVisible(index) }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !reloading else { return }
        gutter.needsDisplay = true
        let row = table.selectedRow
        model.selectedDisplayRow = row >= 0 ? row : nil
        if row >= 0, let block = block(for: row), row - Int(block.start) < block.sourceRows.count {
            model.selectedSourceRow = block.sourceRows[row - Int(block.start)]
        } else { model.selectedSourceRow = nil }
    }

    func moveToMatch(by step: Int) {
        let count = Int(model.current.rowCount)
        guard model.appliedState.search != nil, count > 0 else { return }
        let selected = table.selectedRow
        let row = selected < 0 ? (step > 0 ? 0 : count - 1) : (selected + step + count) % count
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        table.scrollRowToVisible(row)
        if let block = block(for: row) {
            let base = (row - Int(block.start)) * Int(block.nCols)
            if let col = (0..<Int(block.nCols)).first(where: { base + $0 < block.matched.count && block.matched[base + $0] }) {
                scrollToColumn(col)
            }
        }
    }

    func showSelectedColumnFilter() {
        guard let column = model.selectedColumn,
              let index = table.tableColumns.firstIndex(where: { $0.identifier.rawValue == "c\(column)" }) else { return }
        showFilter(at: index)
    }

    private func showFilter(at index: Int) {
        guard let doc = model.doc, let info = model.info, let header = table.headerView,
              index >= 0, index < table.tableColumns.count, let col = columnIndex(table.tableColumns[index]) else { return }
        closePopover()
        let column = UInt32(col)
        let content = ColumnFilterView(doc: doc, column: column, info: info.columns[col], kind: model.kinds[col],
                                       state: model.state, current: model.columnFilters[column] ?? []) { [weak self] filters in
            self?.closePopover()
            self?.model.setFilters(filters, column: column)
        }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(rootView: content)
        popover.show(relativeTo: header.headerRect(ofColumn: index), of: header, preferredEdge: .maxY)
        self.popover = popover
    }

    func closePopover() { popover?.close(); popover = nil }

    @objc func copy(_ sender: Any?) {
        guard let info = model.info, !table.selectedRowIndexes.isEmpty else { return }
        let columns = table.tableColumns.compactMap(columnIndex)
        func escaped(_ value: String) -> String {
            value.contains(where: { "\t\n\r\"".contains($0) })
                ? "\"" + value.replacingOccurrences(of: "\"", with: "\"\"") + "\"" : value
        }
        var lines = [columns.map { escaped(info.columns[$0].name) }.joined(separator: "\t")]
        for row in table.selectedRowIndexes {
            guard let block = block(for: row) else { return }
            let base = (row - Int(block.start)) * Int(block.nCols)
            lines.append(columns.map { escaped(block.cells[base + $0] ?? "") }.joined(separator: "\t"))
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(lines.joined(separator: "\n"), forType: .string)
    }
}

private final class RowCache {
    let blockSize: Int
    private var version: UInt64?
    private var blocks: [Int: RowBlock] = [:]
    init(blockSize: Int) { self.blockSize = blockSize }
    func clear() { blocks.removeAll() }
    func block(for row: Int, version: UInt64, fetch: (Int, Int) -> RowBlock?) -> RowBlock? {
        if self.version != version { clear(); self.version = version }
        let index = row / blockSize
        if let block = blocks[index] { return block }
        guard let block = fetch(index * blockSize, blockSize) else { return nil }
        if blocks.count >= 64 { clear() }
        blocks[index] = block
        return block
    }
}
