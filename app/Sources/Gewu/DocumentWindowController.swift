import AppKit

/// One window showing one file. All data work happens in the Rust core;
/// this class only asks for the rows that are on screen.
final class DocumentWindowController: NSWindowController, NSWindowDelegate,
    NSTableViewDataSource, NSTableViewDelegate, NSToolbarDelegate, NSMenuItemValidation,
    NSSearchFieldDelegate
{
    var onClose: (() -> Void)?

    private let url: URL
    private var doc: GewuDocument?
    private var info: FileInfo?
    private var kinds: [ColumnKind] = []
    private var current = ViewInfo(version: 0, rowCount: 0)

    // What the user asked to see. Sent to Rust as a ViewState.
    private var sort: [SortKey] = []
    /// Filters per column, from that column's filter popover.
    private var columnFilters: [UInt32: [Filter]] = [:]
    private var search: String?

    private let cache = RowCache(blockSize: 256)
    private var viewTask: Task<Void, Never>?
    private var filterColumn = -1
    private var popover: NSPopover?

    private let tableView = NSTableView()
    private let scrollView = NSScrollView()
    private lazy var gutter = RowGutterView(table: tableView)
    private let searchItem = NSSearchToolbarItem(itemIdentifier: DocumentWindowController.searchItemID)
    private var searchField: NSSearchField { searchItem.searchField }
    private let statusLabel = NSTextField(labelWithString: "")
    private lazy var clearFiltersButton: NSButton = {
        let b = NSButton(title: "Clear Filters", target: self, action: #selector(clearAllFilters(_:)))
        b.bezelStyle = .inline
        b.controlSize = .small
        b.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        b.isHidden = true
        return b
    }()
    private let spinner = NSProgressIndicator()
    private let sidebar = SidebarViewController()

    private static let cellID = NSUserInterfaceItemIdentifier("cell")
    private static let headerHeight: CGFloat = 32
    private static let rowViewID = NSUserInterfaceItemIdentifier("stripedRow")
    private static let searchItemID = NSToolbarItem.Identifier("search")

    init(url: URL) {
        self.url = url
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1200, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = url.lastPathComponent
        window.representedURL = url
        window.toolbarStyle = .unified
        super.init(window: window)
        window.delegate = self
        buildLayout()
        window.setContentSize(NSSize(width: 1200, height: 720))
        window.center()
        window.setFrameAutosaveName("GewuDocumentWindow")
        load()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    // MARK: - Layout

    private func buildLayout() {
        guard let window else { return }

        // Toolbar: sidebar toggle on the left, search on the right.
        searchField.placeholderString = "Search all columns"
        searchField.target = self
        searchField.action = #selector(searchChanged(_:))
        searchField.delegate = self
        searchItem.preferredWidthForSearchField = 260
        let toolbar = NSToolbar(identifier: "GewuDocumentToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar

        // Table
        let headerView = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: Self.headerHeight))
        headerView.onFilterClick = { [weak self] index in
            self?.filterColumn = index
            self?.showFilter(nil)
        }
        tableView.headerView = headerView
        // Stripes are painted per row by StripedRowView (see there for why).
        tableView.usesAlternatingRowBackgroundColors = false
        tableView.allowsMultipleSelection = true
        tableView.allowsColumnReordering = true
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.style = .plain
        tableView.rowHeight = 28
        tableView.intercellSpacing = NSSize(width: 0, height: 0)
        tableView.gridStyleMask = []
        tableView.dataSource = self
        tableView.delegate = self


        let scroll = scrollView
        scroll.documentView = tableView
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        // Extends under the toolbar; the system insets the header and rows below it.
        scroll.automaticallyAdjustsContentInsets = true

        // Status bar
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        statusLabel.lineBreakMode = .byTruncatingTail
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false

        let divider = NSBox()
        divider.boxType = .separator
        let status = NSStackView(views: [statusLabel, clearFiltersButton, NSView(), spinner])
        status.orientation = .horizontal
        status.spacing = 6
        status.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)

        let content = NSView()
        for v in [gutter, scroll, divider, status] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        NSLayoutConstraint.activate([
            // The row-number gutter sits left of the table and doesn't scroll
            // sideways with it.
            // Only beside the rows: below the table header, above the status bar.
            gutter.topAnchor.constraint(equalTo: content.safeAreaLayoutGuide.topAnchor, constant: Self.headerHeight),
            gutter.leadingAnchor.constraint(equalTo: content.safeAreaLayoutGuide.leadingAnchor),
            gutter.bottomAnchor.constraint(equalTo: divider.topAnchor),

            scroll.topAnchor.constraint(equalTo: content.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: divider.topAnchor),

            divider.leadingAnchor.constraint(equalTo: content.safeAreaLayoutGuide.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            divider.bottomAnchor.constraint(equalTo: status.topAnchor),

            status.leadingAnchor.constraint(equalTo: content.safeAreaLayoutGuide.leadingAnchor),
            status.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            status.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            status.heightAnchor.constraint(equalToConstant: 26),
        ])
        let contentController = NSViewController()
        contentController.view = content

        // Sidebar + content
        sidebar.onSelectColumn = { [weak self] i in self?.scrollToColumn(i) }
        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebar)
        sidebarItem.minimumThickness = 200
        sidebarItem.maximumThickness = 360
        sidebarItem.canCollapse = true
        let contentItem = NSSplitViewItem(viewController: contentController)
        contentItem.minimumThickness = 400
        if #available(macOS 26.0, *) {
            // The sidebar floats over the table, which scrolls underneath it.
            contentItem.automaticallyAdjustsSafeAreaInsets = true
        }

        let split = NSSplitViewController()
        split.addSplitViewItem(sidebarItem)
        split.addSplitViewItem(contentItem)
        split.splitView.autosaveName = "GewuDocumentSplit"
        window.contentViewController = split
    }

    private func buildColumns(_ info: FileInfo) {
        tableView.tableColumns.forEach(tableView.removeTableColumn)

        // Row numbers live in a gutter beside the table, not in it. They are
        // positions in the file, 0-based like pandas/polars iloc, so they
        // still identify a row after sorting and filtering.
        gutter.fit(largest: max(info.rowCount, 1) - 1)
        gutter.toolTip = "Row in file (0-based)"
        gutter.label = { [weak self] row in
            guard let self, let block = self.block(for: row) else { return nil }
            let r = row - Int(block.start)
            return r >= 0 && r < Int(block.nRows) ? String(block.sourceRows[r]) : nil
        }
        gutter.setShown(Self.showsRowNumbers)

        for (i, column) in info.columns.enumerated() {
            let c = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(i)"))
            let header = ColumnHeaderCell(textCell: column.name)
            header.subtitle = shortTypeName(column.typeName)
            header.showsFunnel = true
            c.headerCell = header
            c.headerToolTip = "\(column.name): \(column.typeName)"
            c.width = fittedWidth(column: i, rows: 0..<min(200, Int(current.rowCount)), maxWidth: 320)
            c.minWidth = 50
            c.sortDescriptorPrototype = NSSortDescriptor(key: "c\(i)", ascending: true)
            tableView.addTableColumn(c)
        }
    }

    // MARK: - Column widths

    private static let cellPadding: CGFloat = 10

    private static func textWidth(_ text: String, _ font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    // MARK: - Row numbers

    private static let showRowNumbersKey = "ShowRowNumbers"

    private static var showsRowNumbers: Bool {
        UserDefaults.standard.object(forKey: showRowNumbersKey) as? Bool ?? true
    }

    @objc func toggleRowNumbers(_ sender: Any?) {
        let show = !Self.showsRowNumbers
        UserDefaults.standard.set(show, forKey: Self.showRowNumbersKey)
        gutter.setShown(show)
    }

    /// Width that fits the header (name and type) and the given rows' text.
    private func fittedWidth(column i: Int, rows: Range<Int>, maxWidth: CGFloat) -> CGFloat {
        guard let info, i < info.columns.count else { return 150 }
        let column = info.columns[i]
        let kind = kinds[i]
        // Header: text insets, the funnel button and room for the sort arrow.
        var width = max(
            Self.textWidth(column.name, .systemFont(ofSize: 12, weight: .semibold)),
            Self.textWidth(shortTypeName(column.typeName), .systemFont(ofSize: 10))
        ) + 10 + 4 + 14 + 8 + 14
        for row in rows {
            guard let block = block(for: row) else { break }
            let r = row - Int(block.start)
            guard r >= 0, r < Int(block.nRows) else { continue }
            let text = block.cells[r * Int(block.nCols) + i]
            let w = text.map { Self.textWidth(String($0.prefix(200)), kind.font) }
                ?? Self.textWidth("null", CellFonts.null)
            width = max(width, w + 2 * Self.cellPadding)
        }
        return min(max(width, 60), maxWidth)
    }

    /// Double-clicking a column divider fits the column to the visible rows.
    func tableView(_ tableView: NSTableView, sizeToFitWidthOfColumn column: Int) -> CGFloat {
        let tableColumn = tableView.tableColumns[column]
        guard let i = columnIndex(tableColumn) else { return tableColumn.width }
        let visible = tableView.rows(in: tableView.visibleRect)
        return fittedWidth(column: i, rows: visible.location..<(visible.location + visible.length), maxWidth: 600)
    }

    private func scrollToColumn(_ i: Int) {
        let index = tableView.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(i)"))
        guard index >= 0 else { return }
        tableView.scrollColumnToVisible(index)
        window?.makeFirstResponder(tableView)
    }

    // MARK: - Toolbar

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.toggleSidebar, .sidebarTrackingSeparator, .flexibleSpace, Self.searchItemID]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier id: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem?
    {
        id == Self.searchItemID ? searchItem : NSToolbarItem(itemIdentifier: id)
    }

    // MARK: - Loading and view updates

    private func load() {
        setBusy(true, "Opening \(url.lastPathComponent)…")
        let path = url.path
        Task {
            do {
                let doc = try await Task.detached { try GewuDocument.open(path: path) }.value
                self.doc = doc
                let info = doc.info()
                self.info = info
                self.kinds = info.columns.map { ColumnKind(typeName: $0.typeName) }
                self.current = doc.currentView()
                buildColumns(info)
                sidebar.show(info: info)
                let rows = NumberFormatter.localizedString(from: NSNumber(value: info.rowCount), number: .decimal)
                let size = ByteCountFormatter.string(fromByteCount: Int64(info.fileSize), countStyle: .file)
                window?.subtitle = "\(rows) rows · \(info.columns.count) columns · \(size)"
                tableView.reloadData()
                setBusy(false)
            } catch {
                setBusy(false, "Could not open file")
                if let window {
                    NSAlert(error: error).beginSheetModal(for: window, completionHandler: nil)
                }
            }
        }
    }

    private func viewState() -> ViewState {
        ViewState(
            sort: sort,
            filters: columnFilters.sorted { $0.key < $1.key }.flatMap(\.value),
            search: search
        )
    }

    /// Ask Rust for a new view. A newer call cancels the older one on both
    /// sides, so rapid clicks never queue up.
    private func refreshView() {
        guard let doc else { return }
        viewTask?.cancel()
        let state = viewState()
        setBusy(true, "Working…")
        viewTask = Task {
            do {
                let view = try await Task.detached { try doc.applyView(state: state) }.value
                guard !Task.isCancelled else { return }
                current = view
                cache.clear()
                tableView.reloadData()
                gutter.needsDisplay = true
                setBusy(false)
            } catch GewuError.Cancelled {
                // Replaced by a newer request.
            } catch {
                setBusy(false, error.localizedDescription)
            }
        }
    }

    private func setBusy(_ busy: Bool, _ message: String? = nil) {
        if busy { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        if let message {
            statusLabel.stringValue = message
            return
        }
        updateStatus()
    }

    /// Row counts, search position and filters, shown in the status bar.
    private func updateStatus() {
        guard let info else { return }
        let shown = NumberFormatter.localizedString(from: NSNumber(value: current.rowCount), number: .decimal)
        let total = NumberFormatter.localizedString(from: NSNumber(value: info.rowCount), number: .decimal)
        var text = current.rowCount == info.rowCount ? "\(total) rows" : "\(shown) of \(total) rows"
        text += " · \(info.columns.count) columns"
        if search != nil, current.rowCount > 0 {
            let selected = tableView.selectedRow
            text += selected >= 0
                ? " · match \(formatCount(UInt64(selected + 1))) of \(shown)"
                : " · ⏎ next match"
        }
        let filtered = columnFilters.count
        if filtered > 0 { text += " · \(filtered) \(filtered == 1 ? "filter" : "filters")" }
        statusLabel.stringValue = text
        clearFiltersButton.isHidden = filtered == 0
    }

    // MARK: - Table data

    func numberOfRows(in tableView: NSTableView) -> Int {
        Int(current.rowCount)
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let view = (tableView.makeView(withIdentifier: Self.rowViewID, owner: self) as? StripedRowView)
            ?? StripedRowView()
        view.identifier = Self.rowViewID
        view.odd = row % 2 == 1
        return view
    }

    private func block(for row: Int) -> RowBlock? {
        guard let doc, let info else { return nil }
        return cache.block(for: row, version: current.version) { start, count in
            try? doc.rows(
                version: current.version,
                start: UInt64(start),
                count: UInt32(count),
                columns: (0..<UInt32(info.columns.count)).map { $0 }
            )
        }
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let tableColumn else { return nil }
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? NSTableCellView)
            ?? makeCell()
        guard let field = cell.textField else { return cell }

        let col = columnIndex(tableColumn) ?? 0
        let kind = col < kinds.count ? kinds[col] : ColumnKind.text
        field.alignment = kind.alignment

        guard let block = block(for: row) else {
            field.stringValue = "…"
            field.textColor = .tertiaryLabelColor
            return cell
        }
        let r = row - Int(block.start)
        guard r >= 0, r < Int(block.nRows) else { return cell }

        let index = r * Int(block.nCols) + col
        let isMatch = index < block.matched.count && block.matched[index]
        // Text matches highlight the matched part; other types the whole value.
        (cell as? MatchCellView)?.wholeCellMatch = isMatch && kind != .text
        if let text = block.cells[index] {
            if isMatch, kind == .text, let search {
                field.attributedStringValue = highlighted(text, query: search, font: kind.font)
            } else {
                field.stringValue = text
                field.font = kind.font
                // Dark text on the light find highlight, in both appearances.
                field.textColor = isMatch ? .black : .labelColor
            }
        } else {
            field.stringValue = "null"
            field.font = CellFonts.null
            field.textColor = .tertiaryLabelColor
        }
        return cell
    }

    private func makeCell() -> NSTableCellView {
        let cell = MatchCellView()
        cell.identifier = Self.cellID
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        field.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: Self.cellPadding),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -Self.cellPadding),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    private func columnIndex(_ column: NSTableColumn) -> Int? {
        let id = column.identifier.rawValue
        guard id.hasPrefix("c") else { return nil }
        return Int(id.dropFirst())
    }

    // MARK: - Sorting

    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        // Like a spreadsheet: the last clicked column is the only sort key.
        if let first = tableView.sortDescriptors.first,
           let key = first.key, key.hasPrefix("c"),
           let i = UInt32(key.dropFirst())
        {
            let key = SortKey(column: i, descending: !first.ascending)
            if tableView.sortDescriptors.count > 1 {
                // Triggers this method again with a single descriptor.
                tableView.sortDescriptors = [first]
                return
            }
            if sort == [key] { return }
            sort = [key]
        } else {
            sort = []
        }
        updateHeaderTitles()
        refreshView()
    }

    // MARK: - Search

    @objc private func searchChanged(_ sender: NSSearchField) {
        let text = sender.stringValue.trimmingCharacters(in: .whitespaces)
        search = text.isEmpty ? nil : text
        refreshView()
    }

    // MARK: - Moving between matches

    /// Return in the search field goes to the next match, Shift-Return to
    /// the previous one. Esc (cancel) clears the search as usual.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard control === searchField, search != nil else { return false }
        if selector == #selector(NSResponder.insertNewline(_:)) {
            let back = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
            back ? findPrevious(nil) : findNext(nil)
            return true
        }
        return false
    }

    @objc func findNext(_ sender: Any?) { moveToMatch(by: 1) }
    @objc func findPrevious(_ sender: Any?) { moveToMatch(by: -1) }

    /// Every row in a searched view matches, so moving selects the next or
    /// previous row and scrolls its first matching cell into view.
    private func moveToMatch(by step: Int) {
        let count = Int(current.rowCount)
        guard search != nil, count > 0 else { return }
        let selected = tableView.selectedRow
        let row = selected < 0 ? (step > 0 ? 0 : count - 1) : (selected + step + count) % count
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        tableView.scrollRowToVisible(row)
        if let block = block(for: row) {
            let r = row - Int(block.start)
            let cols = Int(block.nCols)
            if let c = (0..<cols).first(where: { r * cols + $0 < block.matched.count && block.matched[r * cols + $0] }) {
                let index = tableView.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(c)"))
                if index >= 0 { tableView.scrollColumnToVisible(index) }
            }
        }
        updateStatus()
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        gutter.needsDisplay = true
        if search != nil { updateStatus() }
    }

    @objc func focusSearch(_ sender: Any?) {
        searchItem.beginSearchInteraction()
    }

    // MARK: - Filter (header funnel)

    @objc private func showFilter(_ sender: Any?) {
        guard let doc, let header = tableView.headerView, filterColumn >= 0,
              let col = columnIndex(tableView.tableColumns[filterColumn])
        else { return }
        let column = UInt32(col)
        let current = columnFilters[column] ?? []
        let apply: ([Filter]) -> Void = { [weak self] filters in
            guard let self else { return }
            self.popover?.close()
            self.columnFilters[column] = filters.isEmpty ? nil : filters
            self.updateHeaderTitles()
            self.refreshView()
        }
        // Numbers, dates and times get a range with a histogram; everything
        // else a value list or text search.
        let controller: NSViewController = if kinds[col] != .text, let info, !info.columns[col].typeName.hasPrefix("Interval") {
            RangeFilterController(doc: doc, column: column, info: info.columns[col],
                                  temporal: kinds[col] == .temporal, state: viewState(),
                                  current: current, onApply: apply)
        } else {
            FilterPopoverController(doc: doc, column: column, state: viewState(),
                                    current: current, onApply: apply)
        }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = controller
        popover.show(relativeTo: header.headerRect(ofColumn: filterColumn), of: header, preferredEdge: .maxY)
        self.popover = popover
    }

    @objc func clearAllFilters(_ sender: Any?) {
        guard !columnFilters.isEmpty else { return }
        columnFilters.removeAll()
        updateHeaderTitles()
        refreshView()
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(clearAllFilters(_:)) { return !columnFilters.isEmpty }
        if item.action == #selector(toggleRowNumbers(_:)) {
            item.state = Self.showsRowNumbers ? .on : .off
        }
        return true
    }

    private func updateHeaderTitles() {
        for column in tableView.tableColumns {
            guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
            header.filtered = columnFilters[UInt32(i)] != nil
            let key = sort.first { $0.column == UInt32(i) }
            header.sorted = key != nil
            header.sortAscending = !(key?.descending ?? false)
        }
        tableView.headerView?.needsDisplay = true
        sidebar.setFiltered(Set(columnFilters.keys.map { Int($0) }))
    }

    // MARK: - Copy

    /// Copy selected rows as tab-separated text with a header line.
    @objc func copy(_ sender: Any?) {
        guard let info else { return }
        let columns = tableView.tableColumns.compactMap { columnIndex($0) }
        var lines = [columns.map { info.columns[$0].name }.joined(separator: "\t")]
        for row in tableView.selectedRowIndexes {
            guard let block = block(for: row) else { continue }
            let r = row - Int(block.start)
            let base = r * Int(block.nCols)
            lines.append(columns.map { block.cells[base + $0] ?? "" }.joined(separator: "\t"))
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(lines.joined(separator: "\n"), forType: .string)
    }

    // MARK: - Window

    func windowWillClose(_ notification: Notification) {
        viewTask?.cancel()
        doc?.cancelPending()
        onClose?()
    }
}

/// Rows fetched from Rust in fixed-size blocks, so scrolling makes one call
/// every few screens instead of one per cell.
final class RowCache {
    let blockSize: Int
    private var version: UInt64 = 0
    private var blocks: [Int: RowBlock] = [:]
    private let maxBlocks = 64

    init(blockSize: Int) {
        self.blockSize = blockSize
    }

    func clear() {
        blocks.removeAll()
    }

    func block(for row: Int, version: UInt64, fetch: (Int, Int) -> RowBlock?) -> RowBlock? {
        if version != self.version {
            blocks.removeAll()
            self.version = version
        }
        let index = row / blockSize
        if let block = blocks[index] { return block }
        guard let block = fetch(index * blockSize, blockSize) else { return nil }
        if blocks.count >= maxBlocks { blocks.removeAll() }
        blocks[index] = block
        return block
    }
}
