import AppKit

/// One window showing one file. All data work happens in the Rust core;
/// this class only asks for the rows that are on screen.
final class DocumentWindowController: NSWindowController, NSWindowDelegate,
    NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate, NSToolbarDelegate
{
    var onClose: (() -> Void)?

    private let url: URL
    private var doc: GewuDocument?
    private var info: FileInfo?
    private var kinds: [ColumnKind] = []
    private var current = ViewInfo(version: 0, rowCount: 0)

    // What the user asked to see. Sent to Rust as a ViewState.
    private var sort: [SortKey] = []
    private var valueFilters: [UInt32: [UInt64]] = [:]
    private var search: String?

    private let cache = RowCache(blockSize: 256)
    private var viewTask: Task<Void, Never>?
    private var headerMenuColumn = -1
    private var popover: NSPopover?

    private let tableView = NSTableView()
    private let searchItem = NSSearchToolbarItem(itemIdentifier: DocumentWindowController.searchItemID)
    private var searchField: NSSearchField { searchItem.searchField }
    private let statusLabel = NSTextField(labelWithString: "")
    private let spinner = NSProgressIndicator()
    private let sidebar = SidebarViewController()

    private static let cellID = NSUserInterfaceItemIdentifier("cell")
    private static let rowNumberCellID = NSUserInterfaceItemIdentifier("rowNumberCell")
    private static let rowNumberID = NSUserInterfaceItemIdentifier("row")
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
        searchItem.preferredWidthForSearchField = 260
        let toolbar = NSToolbar(identifier: "GewuDocumentToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar

        // Table
        tableView.headerView = NSTableHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: 32))
        tableView.usesAlternatingRowBackgroundColors = true
        tableView.allowsMultipleSelection = true
        tableView.allowsColumnReordering = true
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.style = .plain
        tableView.rowHeight = 28
        tableView.intercellSpacing = NSSize(width: 0, height: 0)
        tableView.gridStyleMask = []
        tableView.dataSource = self
        tableView.delegate = self

        let headerMenu = NSMenu()
        headerMenu.delegate = self
        tableView.headerView?.menu = headerMenu

        let scroll = NSScrollView()
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
        let status = NSStackView(views: [statusLabel, NSView(), spinner])
        status.orientation = .horizontal
        status.spacing = 6
        status.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)

        let content = NSView()
        for v in [scroll, divider, status] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: content.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: divider.topAnchor),

            divider.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            divider.bottomAnchor.constraint(equalTo: status.topAnchor),

            status.leadingAnchor.constraint(equalTo: content.leadingAnchor),
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

        let split = NSSplitViewController()
        split.addSplitViewItem(sidebarItem)
        split.addSplitViewItem(contentItem)
        split.splitView.autosaveName = "GewuDocumentSplit"
        window.contentViewController = split
    }

    private func buildColumns(_ info: FileInfo) {
        tableView.tableColumns.forEach(tableView.removeTableColumn)

        let rowNumber = NSTableColumn(identifier: Self.rowNumberID)
        let rowHeader = ColumnHeaderCell(textCell: "#")
        rowHeader.rightAligned = true
        rowNumber.headerCell = rowHeader
        rowNumber.width = rowNumberWidth(info)
        rowNumber.headerToolTip = "Row number in the file"
        tableView.addTableColumn(rowNumber)

        for (i, column) in info.columns.enumerated() {
            let c = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(i)"))
            let header = ColumnHeaderCell(textCell: column.name)
            header.subtitle = shortTypeName(column.typeName)
            header.rightAligned = kinds[i] != .text
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

    /// Wide enough for the largest row number in the file.
    private func rowNumberWidth(_ info: FileInfo) -> CGFloat {
        let digits = String(repeating: "8", count: String(info.rowCount).count)
        return max(44, Self.textWidth(digits, CellFonts.rowNumber) + 2 * Self.cellPadding)
    }

    /// Width that fits the header (name and type) and the given rows' text.
    private func fittedWidth(column i: Int, rows: Range<Int>, maxWidth: CGFloat) -> CGFloat {
        guard let info, i < info.columns.count else { return 150 }
        let column = info.columns[i]
        let kind = kinds[i]
        // Header: text inset, sort arrow and room for the filter funnel.
        var width = max(
            Self.textWidth(column.name, .systemFont(ofSize: 12, weight: .semibold)),
            Self.textWidth(shortTypeName(column.typeName), .systemFont(ofSize: 10))
        ) + 16 + 14 + 17
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
        if tableColumn.identifier == Self.rowNumberID, let info { return rowNumberWidth(info) }
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
                window?.subtitle = ByteCountFormatter.string(fromByteCount: Int64(info.fileSize), countStyle: .file)
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
            filters: valueFilters.sorted { $0.key < $1.key }
                .map { .inValues(column: $0.key, keys: $0.value) },
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
        guard let info else { return }
        let shown = NumberFormatter.localizedString(from: NSNumber(value: current.rowCount), number: .decimal)
        let total = NumberFormatter.localizedString(from: NSNumber(value: info.rowCount), number: .decimal)
        var text = current.rowCount == info.rowCount ? "\(total) rows" : "\(shown) of \(total) rows"
        text += " · \(info.columns.count) columns"
        if !valueFilters.isEmpty { text += " · \(valueFilters.count) filtered" }
        statusLabel.stringValue = text
    }

    // MARK: - Table data

    func numberOfRows(in tableView: NSTableView) -> Int {
        Int(current.rowCount)
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
        let isRowNumber = tableColumn.identifier == Self.rowNumberID
        let id = isRowNumber ? Self.rowNumberCellID : Self.cellID
        let cell = (tableView.makeView(withIdentifier: id, owner: self) as? NSTableCellView)
            ?? makeCell(id)
        guard let field = cell.textField else { return cell }

        let col = columnIndex(tableColumn) ?? 0
        let kind = isRowNumber || col >= kinds.count ? ColumnKind.number : kinds[col]
        field.alignment = kind.alignment

        guard let block = block(for: row) else {
            field.stringValue = "…"
            field.textColor = .tertiaryLabelColor
            return cell
        }
        let r = row - Int(block.start)
        guard r >= 0, r < Int(block.nRows) else { return cell }

        if isRowNumber {
            field.stringValue = String(block.sourceRows[r] + 1)
            return cell
        }

        if let text = block.cells[r * Int(block.nCols) + col] {
            field.stringValue = text
            field.font = kind.font
            field.textColor = .labelColor
        } else {
            field.stringValue = "null"
            field.font = CellFonts.null
            field.textColor = .tertiaryLabelColor
        }
        return cell
    }

    private func makeCell(_ id: NSUserInterfaceItemIdentifier) -> NSTableCellView {
        let isRowNumber = id == Self.rowNumberCellID
        let cell = isRowNumber ? RowNumberCellView() : NSTableCellView()
        cell.identifier = id
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        if isRowNumber {
            field.font = CellFonts.rowNumber
            field.textColor = .secondaryLabelColor
        }
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

    @objc func focusSearch(_ sender: Any?) {
        searchItem.beginSearchInteraction()
    }

    // MARK: - Filter (header right-click)

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let header = tableView.headerView, let event = NSApp.currentEvent else { return }
        let point = header.convert(event.locationInWindow, from: nil)
        headerMenuColumn = header.column(at: point)
        guard headerMenuColumn >= 0,
              let col = columnIndex(tableView.tableColumns[headerMenuColumn])
        else { return }

        let filter = NSMenuItem(title: "Filter…", action: #selector(showFilter(_:)), keyEquivalent: "")
        filter.target = self
        menu.addItem(filter)

        if valueFilters[UInt32(col)] != nil {
            let clear = NSMenuItem(title: "Clear Filter", action: #selector(clearColumnFilter(_:)), keyEquivalent: "")
            clear.target = self
            menu.addItem(clear)
        }
        if !valueFilters.isEmpty {
            let clearAll = NSMenuItem(title: "Clear All Filters", action: #selector(clearAllFilters(_:)), keyEquivalent: "")
            clearAll.target = self
            menu.addItem(clearAll)
        }
    }

    @objc private func showFilter(_ sender: Any?) {
        guard let doc, let header = tableView.headerView, headerMenuColumn >= 0,
              let col = columnIndex(tableView.tableColumns[headerMenuColumn])
        else { return }
        let column = UInt32(col)
        let selected = valueFilters[column].map { Set($0) }

        let controller = FilterPopoverController(
            doc: doc, column: column, state: viewState(), selected: selected
        ) { [weak self] keys in
            guard let self else { return }
            self.popover?.close()
            if let keys {
                self.valueFilters[column] = Array(keys).sorted()
            } else {
                self.valueFilters[column] = nil
            }
            self.updateHeaderTitles()
            self.refreshView()
        }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = controller
        popover.show(relativeTo: header.headerRect(ofColumn: headerMenuColumn), of: header, preferredEdge: .maxY)
        self.popover = popover
    }

    @objc private func clearColumnFilter(_ sender: Any?) {
        guard headerMenuColumn >= 0, let col = columnIndex(tableView.tableColumns[headerMenuColumn]) else { return }
        valueFilters[UInt32(col)] = nil
        updateHeaderTitles()
        refreshView()
    }

    @objc private func clearAllFilters(_ sender: Any?) {
        valueFilters.removeAll()
        updateHeaderTitles()
        refreshView()
    }

    private func updateHeaderTitles() {
        for column in tableView.tableColumns {
            guard let i = columnIndex(column), let header = column.headerCell as? ColumnHeaderCell else { continue }
            header.filtered = valueFilters[UInt32(i)] != nil
            header.sorted = sort.contains { $0.column == UInt32(i) }
        }
        tableView.headerView?.needsDisplay = true
        sidebar.setFiltered(Set(valueFilters.keys.map { Int($0) }))
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
