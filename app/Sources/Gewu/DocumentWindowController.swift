import AppKit

/// One window showing one file. All data work happens in the Rust core;
/// this class only asks for the rows that are on screen.
final class DocumentWindowController: NSWindowController, NSWindowDelegate,
    NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate
{
    var onClose: (() -> Void)?

    private let url: URL
    private var doc: GewuDocument?
    private var info: FileInfo?
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
    private let searchField = NSSearchField()
    private let statusLabel = NSTextField(labelWithString: "")
    private let spinner = NSProgressIndicator()

    private static let cellID = NSUserInterfaceItemIdentifier("cell")
    private static let rowNumberID = NSUserInterfaceItemIdentifier("row")

    init(url: URL) {
        self.url = url
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = url.lastPathComponent
        window.representedURL = url
        window.setFrameAutosaveName("GewuDocumentWindow")
        window.center()
        super.init(window: window)
        window.delegate = self
        buildLayout()
        load()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    // MARK: - Layout

    private func buildLayout() {
        guard let content = window?.contentView else { return }

        searchField.placeholderString = "Search all columns"
        searchField.target = self
        searchField.action = #selector(searchChanged(_:))

        tableView.usesAlternatingRowBackgroundColors = true
        tableView.allowsMultipleSelection = true
        tableView.allowsColumnReordering = true
        tableView.columnAutoresizingStyle = .noColumnAutoresizing
        tableView.style = .plain
        tableView.rowHeight = 20
        tableView.gridStyleMask = [.solidVerticalGridLineMask]
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

        statusLabel.textColor = .secondaryLabelColor
        statusLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false

        let status = NSStackView(views: [statusLabel, spinner])
        status.orientation = .horizontal
        status.spacing = 6

        for v in [searchField, scroll, status] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        NSLayoutConstraint.activate([
            searchField.topAnchor.constraint(equalTo: content.topAnchor, constant: 8),
            searchField.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -10),
            searchField.widthAnchor.constraint(equalToConstant: 260),

            scroll.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 8),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: status.topAnchor, constant: -4),

            status.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 10),
            status.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -6),
        ])
    }

    private func buildColumns(_ info: FileInfo) {
        tableView.tableColumns.forEach(tableView.removeTableColumn)

        let rowNumber = NSTableColumn(identifier: Self.rowNumberID)
        rowNumber.title = "#"
        rowNumber.width = 60
        rowNumber.headerToolTip = "Row number in the file"
        tableView.addTableColumn(rowNumber)

        for (i, column) in info.columns.enumerated() {
            let c = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c\(i)"))
            c.title = column.name
            c.headerToolTip = "\(column.name): \(column.typeName)"
            c.width = 140
            c.minWidth = 40
            c.sortDescriptorPrototype = NSSortDescriptor(key: "c\(i)", ascending: true)
            tableView.addTableColumn(c)
        }
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
                self.current = doc.currentView()
                buildColumns(info)
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
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? NSTableCellView)
            ?? makeCell()
        guard let field = cell.textField else { return cell }

        guard let block = block(for: row) else {
            field.stringValue = "…"
            field.textColor = .tertiaryLabelColor
            return cell
        }
        let r = row - Int(block.start)
        guard r >= 0, r < Int(block.nRows) else { return cell }

        if tableColumn.identifier == Self.rowNumberID {
            field.stringValue = String(block.sourceRows[r] + 1)
            field.textColor = .secondaryLabelColor
            field.alignment = .right
            return cell
        }

        field.alignment = .left
        let col = columnIndex(tableColumn) ?? 0
        if let text = block.cells[r * Int(block.nCols) + col] {
            field.stringValue = text
            field.textColor = .labelColor
        } else {
            field.stringValue = "null"
            field.textColor = .tertiaryLabelColor
        }
        return cell
    }

    private func makeCell() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = Self.cellID
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        field.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(field)
        cell.textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
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
        refreshView()
    }

    // MARK: - Search

    @objc private func searchChanged(_ sender: NSSearchField) {
        let text = sender.stringValue.trimmingCharacters(in: .whitespaces)
        search = text.isEmpty ? nil : text
        refreshView()
    }

    @objc func focusSearch(_ sender: Any?) {
        window?.makeFirstResponder(searchField)
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
        guard let info else { return }
        for column in tableView.tableColumns {
            guard let i = columnIndex(column) else { continue }
            let name = info.columns[i].name
            column.title = valueFilters[UInt32(i)] != nil ? "\(name) ⏷" : name
        }
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
