import AppKit
import GewuKit
import SwiftUI

extension TableViewController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int { shownRows.count }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let result = (table.makeView(withIdentifier: rowViewID, owner: self) as? StripedRowView) ?? StripedRowView()
        result.identifier = rowViewID
        if !result.wantsLayer { result.wantsLayer = true }
        result.odd = row % 2 == 1
        return result
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let tableColumn, let column = columnID(tableColumn) else { return nil }
        let cell =
            (table.makeView(withIdentifier: tableColumn.identifier, owner: self) as? MatchCellView)
            ?? MatchCellView.make(identifier: tableColumn.identifier)
        bind(cell, column: column, row: row)
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !switchingRows else { return }
        gutter.needsDisplay = true
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
        if let id = columnID(at: column) {
            // Start after AppKit finishes its divider resize notification.
            Task { self.fitColumns([id], automatic: false) }
        }
        return table.tableColumns[column].width
    }

    func tableViewColumnDidResize(_ notification: Notification) {
        guard !applyingWidths, let column = notification.userInfo?["NSTableColumn"] as? NSTableColumn,
            let id = columnID(column)
        else { return }
        widths.manual(id)
    }

    func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
        guard let column = columnID(tableColumn) else { return }
        let multiple = (table.headerView as? FilterHeaderView)?.sortingMultiple ?? false
        // Let AppKit finish its own header tracking before the table changes.
        Task {
            self.model.select(column)
            self.model.cycleSort(column: column, multiple: multiple)
        }
    }

    // MARK: Cells

    /// Show a block that arrived for the shown rows in the visible cells it covers.
    func display(_ blocks: [BlockKey: ColumnText]) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        let visibleRows = visible.location..<(visible.location + visible.length)
        let visibleColumns = table.columnIndexes(in: table.visibleRect)
        for (key, text) in blocks {
            let index = table.column(withIdentifier: identifier(for: key.column))
            guard visibleColumns.contains(index) else { continue }
            for row in key.rows.clamped(to: visibleRows) {
                if let cell = table.view(atColumn: index, row: row, makeIfNecessary: true) as? MatchCellView {
                    render(cell, column: key.column, row: row, text: text.cell(row))
                }
            }
        }
    }

    /// Refresh visible cells from their content: rebind cells of other rows,
    /// fill loaded ones and update Find decoration. Cells in `skipping` were
    /// bound in this viewport already.
    func refreshVisibleCells(restyle: Bool, skipping bound: Viewport = .empty) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        for index in table.columnIndexes(in: table.visibleRect) {
            guard let column = columnID(table.tableColumns[index]) else { continue }
            for row in visible.location..<(visible.location + visible.length) {
                if bound.rows.contains(row), bound.columns.contains(column) { continue }
                guard let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView else {
                    continue
                }
                if cell.boundRows !== shownRows {
                    bind(cell, column: column, row: row)
                } else if cell.isLoading, let text = texts.text(row: row, column: column) {
                    render(cell, column: column, row: row, text: text.cell(row))
                } else {
                    let kind = model.column(column)?.valueCategory ?? .text
                    if restyle {
                        cell.updateStyle(
                            font: cell.muted && !cell.isLoading ? fonts.null : fonts.font(for: kind),
                            alignment: kind.alignment)
                    }
                    let decoration = findDecoration(row: row, column: column, kind: kind, loaded: !cell.isLoading)
                    cell.updateFind(query: decoration.query, wholeMatch: decoration.whole, current: decoration.current)
                }
            }
        }
    }

    func refreshCellAppearance() {
        table.enumerateAvailableRowViews { row, _ in
            for cell in row.subviews.compactMap({ $0 as? MatchCellView }) { cell.refreshAppearance() }
        }
    }

    private func bind(_ cell: MatchCellView, column: ColumnID, row: Int) {
        cell.boundRows = shownRows
        if let text = texts.text(row: row, column: column) {
            render(cell, column: column, row: row, text: text.cell(row))
        } else {
            let kind = model.column(column)?.valueCategory ?? .text
            cell.isLoading = true
            cell.configure(text: "…", font: fonts.font(for: kind), alignment: kind.alignment, muted: true)
        }
    }

    private func render(_ cell: MatchCellView, column: ColumnID, row: Int, text: String?) {
        let kind = model.column(column)?.valueCategory ?? .text
        let decoration = findDecoration(row: row, column: column, kind: kind, loaded: true)
        cell.boundRows = shownRows
        cell.isLoading = false
        cell.configure(
            text: text ?? "null", font: text == nil ? fonts.null : fonts.font(for: kind), alignment: kind.alignment,
            query: decoration.query, wholeMatch: decoration.whole, current: decoration.current, muted: text == nil)
    }

    /// Find marks a matched text cell's substrings, or the whole value of other cells.
    private func findDecoration(row: Int, column: ColumnID, kind: ValueCategory, loaded: Bool)
        -> (query: String?, whole: Bool, current: Bool)
    {
        let find = presentation.find
        // Matches describe the presented rows; never mark cells of rows being replaced.
        let hit = loaded && presentation.rows === shownRows && find.highlights.isMatch(row: row, column: column)
        let current = find.currentMatch == CellMatch(displayRow: row, column: column)
        return (hit && kind == .text ? find.query : nil, hit && kind != .text, current)
    }

    // MARK: Cell menu and preview

    private struct CellContent {
        let row: Int
        let column: ColumnID
        let text: String?
    }

    private func cellContent(row: Int, index: Int) -> CellContent? {
        guard row >= 0, let column = columnID(at: index),
            // Visible blocks stay available even when they are too large for the cache.
            let text = texts.text(row: row, column: column)
        else { return nil }
        return CellContent(row: row, column: column, text: text.cell(row))
    }

    func cellMenu(row: Int, index: Int) -> NSMenu? {
        guard let content = cellContent(row: row, index: index) else { return nil }
        model.select(content.column)
        if !table.selectedRowIndexes.contains(row) {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        let menu = NSMenu()
        menu.addItem(menuItem("Copy Cell", #selector(copyCell(_:)), content))
        menu.addItem(menuItem("Copy Selected Rows", #selector(copy(_:)), content))
        menu.addItem(menuItem("View Cell…", #selector(viewCell(_:)), content))
        menu.addItem(.separator())
        menu.addItem(menuItem("Filter Column…", #selector(filterMenuColumn(_:)), content.column))
        if model.filteredColumns.contains(content.column) {
            menu.addItem(menuItem("Clear Column Filter", #selector(clearMenuColumnFilter(_:)), content.column))
        }
        menu.addItem(menuItem("Analyze Column", #selector(analyzeMenuColumn(_:)), content.column))
        return menu
    }

    func menuItem(_ title: String, _ action: Selector, _ value: Any) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        item.representedObject = value
        return item
    }

    @objc private func copyCell(_ item: NSMenuItem) {
        guard let content = item.representedObject as? CellContent else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(content.text ?? "", forType: .string)
    }

    @objc private func viewCell(_ item: NSMenuItem) {
        if let content = item.representedObject as? CellContent { showCell(content) }
    }

    @objc func viewClickedCell(_ sender: Any?) {
        if let content = cellContent(row: table.clickedRow, index: table.clickedColumn) { showCell(content) }
    }

    private func showCell(_ content: CellContent) {
        cellPopover?.close()
        let index = table.column(withIdentifier: identifier(for: content.column))
        guard index >= 0 else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(
            rootView: CellPreview(name: model.column(content.column)?.name ?? "", text: content.text))
        popover.show(relativeTo: table.frameOfCell(atColumn: index, row: content.row), of: table, preferredEdge: .maxY)
        cellPopover = popover
    }
}

extension MatchCellView {
    static func make(identifier: NSUserInterfaceItemIdentifier) -> MatchCellView {
        let cell = MatchCellView()
        cell.identifier = identifier
        // Reused text fields can briefly retain their old width during layout.
        // macOS 14+ views do not clip descendants by default.
        cell.clipsToBounds = true
        let field = NSTextField(labelWithString: "")
        field.clipsToBounds = true
        field.lineBreakMode = .byTruncatingTail
        field.translatesAutoresizingMaskIntoConstraints = false
        field.cell?.setAccessibilityElement(false)
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -10),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
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
