import AppKit
import SwiftUI

/// The native table consumes its own presentation, independent of window panels
/// and pending queries. Find updates decorate existing cells.
struct DataTable: View {
    let model: DocumentModel
    var showsRowNumbers: Bool
    @AppStorage("TableFontSize") private var fontSize = 12.0
    @AppStorage("TableRowHeight") private var rowHeight = 28.0
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    var body: some View {
        let input = model.tableInput
        DataTableRepresentable(
            model: model,
            input: input,
            find: FindDecoration(
                revision: model.find.revision, highlightRevision: model.find.highlightRevision,
                currentMatch: model.find.currentMatch),
            showsRowNumbers: showsRowNumbers,
            fontSize: min(max(fontSize, 10), 20),
            rowHeight: min(max(rowHeight, 24), 36),
            rowNumberBase: rowNumberBase
        )
    }
}

/// The Find state that changes how existing cells are decorated.
private struct FindDecoration: Equatable {
    let revision: UInt64
    let highlightRevision: UInt64
    let currentMatch: CellMatch?
}

private struct DataTableRepresentable: NSViewControllerRepresentable {
    let model: DocumentModel
    let input: DocumentModel.TableInput
    // Read in `DataTable.body` so SwiftUI calls `update` when Find state or highlights change.
    let find: FindDecoration
    let showsRowNumbers: Bool
    let fontSize: CGFloat
    let rowHeight: CGFloat
    let rowNumberBase: Int

    func makeNSViewController(context: Context) -> DataTableController {
        let controller = DataTableController(model: model)
        model.table = controller
        return controller
    }

    func updateNSViewController(_ controller: DataTableController, context: Context) {
        controller.update(
            input: input,
            showsRowNumbers: showsRowNumbers,
            fontSize: fontSize,
            rowHeight: rowHeight,
            rowNumberBase: rowNumberBase)
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
    private let segments: TableSegmentLoader
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
    private var columnByIdentifier: [NSUserInterfaceItemIdentifier: ColumnId] = [:]
    private var displayedInput: DocumentModel.TableInput?
    private var displayedView: GewuOrderedView? { displayedInput?.ordered }
    private var viewportRows: NSRange?
    private var viewportColumns: [ColumnId] = []
    private var showsNumbers: Bool?
    private var reloading = false
    private var textSize: CGFloat = 12
    private var fonts = CellFonts()
    private var rowNumberBase = 0
    private let rowHeaderID = NSUserInterfaceItemIdentifier("rowHeader")
    private let rowViewID = NSUserInterfaceItemIdentifier("stripedRow")

    init(model: DocumentModel, loader: TableSegmentLoader? = nil) {
        self.model = model
        segments = loader ?? TableSegmentLoader()
        super.init(nibName: nil, bundle: nil)
        segments.onLoad = { [weak self] view, loaded in
            guard let self, self.displayedView === view, self.model.ordered === view else { return }
            self.display(loaded)
        }
        segments.onError = { [weak self] view, error in
            guard let self, self.displayedView === view, self.model.ordered === view else { return }
            self.model.reportError(error, prefix: "Could not load rows: ")
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private func identifier(for id: ColumnId) -> NSUserInterfaceItemIdentifier {
        NSUserInterfaceItemIdentifier("c\(id.raw)")
    }

    private func columnId(_ column: NSTableColumn) -> ColumnId? {
        columnByIdentifier[column.identifier]
    }

    /// Predict the visible region after row-count and column-layout changes.
    /// Include one bordering row for fractional scrolling and AppKit's clipping.
    private func viewport(
        for ordered: GewuOrderedView, columns visibleCols: [ColumnId], sameLayout: Bool
    ) -> TableSegmentLoader.Viewport {
        let count = Int(clamping: ordered.rowCount())
        let clip = scroll.contentView.bounds
        let stride = table.rowHeight + table.intercellSpacing.height
        let y = max(0, min(clip.minY, CGFloat(count) * stride - clip.height))
        let first = max(0, Int(floor(y / stride)) - 1)
        let last = min(count, Int(ceil((y + clip.height) / stride)) + 1)
        let rows = min(first, count)..<max(min(first, count), last)
        if sameLayout {
            let columns = table.columnIndexes(in: clip).compactMap { index -> ColumnId? in
                let column = table.tableColumns[index]
                guard !column.isHidden else { return nil }
                return columnId(column)
            }
            return .init(rows: rows, columns: columns)
        }
        let gutterWidth = showsNumbers == true ? gutter.fittedWidth : 0
        let widths = visibleCols.map { col in
            table.tableColumn(withIdentifier: identifier(for: col))?.width ?? 140
        }
        let totalWidth = widths.reduce(gutterWidth, +)
        let x = max(0, min(clip.minX, totalWidth - clip.width))
        var edge = gutterWidth
        var columns: [ColumnId] = []
        for (col, width) in zip(visibleCols, widths) {
            if edge + width > x && edge < x + clip.width { columns.append(col) }
            edge += width
        }
        return .init(rows: rows, columns: columns)
    }

    func loadViewport(in ordered: GewuOrderedView, columns: [ColumnId]) async throws -> TableSegmentLoader.Snapshot {
        let sameLayout = displayedInput?.visibleColumns == columns
        return try await segments.snapshot(of: ordered) {
            viewport(for: ordered, columns: columns, sameLayout: sameLayout)
        }
    }

    func commit(_ snapshot: TableSegmentLoader.Snapshot) {
        loadViewIfNeeded()
        closePopover()
        stopFitting()
        let rowCountChanged = displayedView?.rowCount() != snapshot.view.rowCount()
        let input = model.tableInput
        let columnsChanged = displayedInput?.visibleColumns != input.visibleColumns
        reloading = true
        segments.install(snapshot)
        updateColumns(input)
        table.deselectAll(nil)
        if rowCountChanged { table.noteNumberOfRowsChanged() }
        if rowCountChanged || columnsChanged { table.layoutSubtreeIfNeeded() }
        // A retained offscreen cell binds to this View when it next enters the viewport.
        display(snapshot.segments)
        viewportRows = NSRange(snapshot.viewport.rows)
        viewportColumns = snapshot.viewport.columns
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
        let initial = model.tableInput
        segments.reset(to: initial.ordered)
        updateColumns(initial)
        table.noteNumberOfRowsChanged()
    }

    /// SwiftUI updates presentation; replacing row data goes through commit.
    func update(
        input: DocumentModel.TableInput? = nil,
        showsRowNumbers: Bool,
        fontSize: CGFloat = 12,
        rowHeight: CGFloat = 28,
        rowNumberBase: Int = 0
    ) {
        loadViewIfNeeded()
        let resolvedInput = input ?? model.tableInput
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
        let columnsChanged = displayedInput?.visibleColumns != resolvedInput.visibleColumns
        updateColumns(resolvedInput, restyle: presentationChanged)
        if columnsChanged {
            table.layoutSubtreeIfNeeded()
        }
        if presentationChanged || findChanged {
            refreshVisibleCells(restyle: presentationChanged)
        }
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    private func applyColumnLayout(_ visible: [ColumnId]) {
        applyingColumns = true
        let visibleSet = Set(visible)
        for column in table.tableColumns {
            if let id = columnId(column) { column.isHidden = !visibleSet.contains(id) }
        }
        for (offset, id) in visible.enumerated() {
            let from = table.column(withIdentifier: identifier(for: id))
            if from != offset + 1 { table.moveColumn(from, toColumn: offset + 1) }
        }
        applyingColumns = false
    }

    func syncColumns() {
        guard builtColumns, displayedColumns != model.visibleColumns else { return }
        applyColumnLayout(model.visibleColumns)
        table.layoutSubtreeIfNeeded()
        updateViewport()
        requestHighlights()
    }

    private func updateColumns(_ input: DocumentModel.TableInput, restyle: Bool = false) {
        let previous = displayedInput
        displayedInput = input
        let visible = input.visibleColumns
        if previous?.visibleColumns != visible {
            applyColumnLayout(visible)
        }
        if previous != input || restyle {
            let filters = Dictionary(uniqueKeysWithValues: input.filters.map { ($0.column, $0.filter) })
            let oldFilters = Dictionary(uniqueKeysWithValues: (previous?.filters ?? []).map { ($0.column, $0.filter) })
            let sorts = Dictionary(
                uniqueKeysWithValues: input.sorts.enumerated().map { ($1.column, (sort: $1, priority: $0)) })
            let oldSorts = Dictionary(
                uniqueKeysWithValues: (previous?.sorts ?? []).enumerated().map { ($1.column, (sort: $1, priority: $0)) }
            )
            let multipleSortKeys = input.sorts.count > 1
            let hadMultipleSortKeys = (previous?.sorts.count ?? 0) > 1
            for column in table.tableColumns {
                guard let id = columnId(column),
                    let info = model.columnInfo[id],
                    let header = column.headerCell as? ColumnHeaderCell
                else { continue }
                let oldSort = oldSorts[id]
                let newSort = sorts[id]
                if previous != nil, !restyle,
                    oldFilters[id] == filters[id],
                    oldSort?.sort == newSort?.sort,
                    oldSort?.priority == newSort?.priority,
                    multipleSortKeys == hadMultipleSortKeys,
                    (previous?.selectedColumn == id) == (input.selectedColumn == id),
                    (previous?.inspectedColumn == id) == (input.inspectedColumn == id)
                {
                    continue
                }
                header.textSize = textSize
                let filter = filters[id]
                header.filtered = filter != nil
                header.subtitle = info.displayType
                column.headerToolTip =
                    "\(info.name): \(info.technicalType)"
                    + (filter.map { "\n\($0.detailSummary)" } ?? "")
                    + "\nClick to cycle sorting; Shift-click for multiple keys. Use the funnel to filter."
                    + "\nDrag to reorder; double-click a divider to fit the column."
                header.inspected = input.inspectedColumn == id
                header.active = input.selectedColumn == id
                header.sorted = newSort != nil
                header.sortPriority = multipleSortKeys ? (newSort.map { $0.priority + 1 } ?? 0) : 0
                header.sortAscending = !(newSort?.sort.descending ?? false)
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
            segments.updateViewport(rows: 0..<0, columns: [])
            return
        }
        let columns = table.columnIndexes(in: table.visibleRect).compactMap { index -> ColumnId? in
            let column = table.tableColumns[index]
            guard !column.isHidden else { return nil }
            return columnId(column)
        }
        if viewportRows != visibleRows || viewportColumns != columns {
            refreshVisibleCells(restyle: true, enteringViewport: true)
            viewportRows = visibleRows
            viewportColumns = columns
        }
        segments.updateViewport(
            rows: visibleRows.location..<(visibleRows.location + visibleRows.length), columns: columns)
    }

    /// Request match flags only for the rows and columns on screen.
    private func requestHighlights() {
        let rows = table.rows(in: table.visibleRect)
        guard rows.location != NSNotFound, rows.length > 0 else { return }
        let columns = table.columnIndexes(in: table.visibleRect).compactMap { index -> ColumnId? in
            let column = table.tableColumns[index]
            return column.isHidden ? nil : columnId(column)
        }
        model.find.requestHighlight(rows: rows.location..<(rows.location + rows.length), columns: columns)
    }

    private func buildColumns(_ info: DatasetInfo) {
        gutter.label = { [weak self] row in
            guard let self, let source = self.segments.sourceRow(for: row) else { return nil }
            return formatRowNumber(source, base: self.rowNumberBase)
        }
        let rowHeader = NSTableColumn(identifier: rowHeaderID)
        rowHeader.headerCell = ColumnHeaderCell(textCell: "")
        rowHeader.width = gutter.fittedWidth
        rowHeader.minWidth = gutter.fittedWidth
        rowHeader.maxWidth = gutter.fittedWidth
        rowHeader.resizingMask = []
        table.addTableColumn(rowHeader)
        fitGutter(info)
        for column in info.columns {
            let itemId = identifier(for: column.id)
            columnByIdentifier[itemId] = column.id
            let c = NSTableColumn(identifier: itemId)
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

    /// Render segments directly, including segments too large for the cache.
    private func display(_ loaded: TableSegments) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        let visibleRows = visible.location..<(visible.location + visible.length)
        let visibleColumns = table.columnIndexes(in: table.visibleRect)
        for (key, segment) in loaded.segments {
            guard let column = key.column else { continue }
            let rows = key.rows.clamped(to: visibleRows)
            let index = table.column(withIdentifier: identifier(for: column))
            guard !rows.isEmpty, visibleColumns.contains(index) else { continue }
            for row in rows {
                if let cell = table.view(atColumn: index, row: row, makeIfNecessary: true) as? MatchCellView {
                    render(cell, column: column, row: row, text: segment.cell(row: row - key.start))
                }
            }
        }
        guard !loaded.sourceRows.isEmpty else { return }
        gutter.needsDisplay = true
    }

    /// Decoration and style changes operate on the content already in the cell,
    /// including large values intentionally not retained by the row cache.
    private func refreshVisibleCells(restyle: Bool, enteringViewport: Bool = false) {
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else { return }
        for index in table.columnIndexes(in: table.visibleRect) {
            guard let column = columnId(table.tableColumns[index]) else { continue }
            for row in visible.location..<(visible.location + visible.length) {
                if enteringViewport, let viewportRows, NSLocationInRange(row, viewportRows),
                    viewportColumns.contains(column)
                {
                    continue
                }
                guard let cell = table.view(atColumn: index, row: row, makeIfNecessary: false) as? MatchCellView else {
                    continue
                }
                if cell.boundView !== displayedView {
                    bind(cell, column: column, row: row)
                } else if cell.isLoading, let cached = segments.segment(for: row, column: column) {
                    render(cell, column: column, row: row, text: cached.segment.cell(row: cached.row))
                } else {
                    let kind = model.columnInfo[column]?.valueCategory ?? .text
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
                        current: current?.displayRow == UInt64(row) && current?.column == column)
                }
            }
        }
    }

    func stopLoading() {
        segments.stop()
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
        guard let tableColumn, let col = columnId(tableColumn) else { return nil }
        let cell =
            (table.makeView(withIdentifier: tableColumn.identifier, owner: self) as? MatchCellView)
            ?? makeCell(identifier: tableColumn.identifier)
        bind(cell, column: col, row: row)
        return cell
    }

    private func bind(_ cell: MatchCellView, column: ColumnId, row: Int) {
        cell.boundView = displayedView
        if let cached = segments.segment(for: row, column: column) {
            render(cell, column: column, row: row, text: cached.segment.cell(row: cached.row))
        } else {
            let kind = model.columnInfo[column]?.valueCategory ?? .text
            cell.isLoading = true
            cell.configure(text: "…", font: fonts.font(for: kind), alignment: kind.alignment, muted: true)
        }
    }

    private func render(_ cell: MatchCellView, column: ColumnId, row: Int, text: String?) {
        let kind = model.columnInfo[column]?.valueCategory ?? .text
        let hit = model.find.isMatch(row: row, column: column)
        let current = model.find.currentMatch
        cell.boundView = displayedView
        cell.isLoading = false
        cell.configure(
            text: text ?? "null", font: text == nil ? fonts.null : fonts.font(for: kind), alignment: kind.alignment,
            query: hit && kind == .text ? model.find.activeQuery : nil, wholeMatch: hit && kind != .text,
            current: current?.displayRow == UInt64(row) && current?.column == column, muted: text == nil)
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
        let column: ColumnId
        let text: String?
    }

    private func cellContent(row: Int, index: Int) -> CellContent? {
        guard row >= 0, index >= 0, index < table.tableColumns.count,
            let column = columnId(table.tableColumns[index])
        else { return nil }
        if let cached = segments.segment(for: row, column: column) {
            return CellContent(row: row, column: column, text: cached.segment.cell(row: cached.row))
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
                !model.appliedFilterColumns.contains(content.column)
            {
                continue
            }
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = content.column
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
        let index = table.column(withIdentifier: identifier(for: content.column))
        guard index >= 0 else { return }
        let panel = NSPopover()
        panel.behavior = .transient
        panel.contentViewController = NSHostingController(
            rootView: CellPreview(
                name: model.columnInfo[content.column]?.name ?? "", text: content.text))
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
        if let source = columnId(table.tableColumns[column]) {
            // Start after AppKit finishes its divider resize notification.
            DispatchQueue.main.async { [weak self] in self?.fitColumns([source], automatic: false) }
        }
        return table.tableColumns[column].width
    }

    func tableViewColumnDidResize(_ notification: Notification) {
        guard !applyingWidths,
            let column = notification.userInfo?["NSTableColumn"] as? NSTableColumn,
            let source = columnId(column)
        else { return }
        widths.manual(source)
    }

    private func fitVisibleColumns() {
        guard builtColumns, !fittingAutomatically, view.window != nil, displayedView != nil else { return }
        let columns = table.columnIndexes(in: table.visibleRect).compactMap { index -> ColumnId? in
            let col = table.tableColumns[index]
            guard !col.isHidden, let source = columnId(col), widths.needsAutomatic(source) else {
                return nil
            }
            return source
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

    private func fitColumns(_ columns: [ColumnId], automatic: Bool) {
        guard let ordered = displayedView, !columns.isEmpty else { return }
        if automatic { fittingAutomatically = true } else { fitTask?.cancel() }
        let tickets = columns.map { widths.begin($0) }
        let rows = table.rows(in: table.visibleRect)
        let start = rows.location == NSNotFound ? 0 : UInt64(rows.location)
        let inputs: [ColumnWidthMeasurement.Input] = columns.compactMap { column in
            guard let info = model.columnInfo[column] else { return nil }
            let sample = self.segments.segment(for: Int(start), column: column)
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
                let result: [ColumnId: CGFloat] = try await withTaskCancellationHandler {
                    try Task.checkCancellation()
                    return try await withCheckedThrowingContinuation { continuation in
                        worker.async {
                            do {
                                continuation.resume(
                                    returning: try ColumnWidthMeasurement.measure(
                                        ordered: ordered, columns: inputs, start: start, fontSize: fontSize,
                                        maximum: automatic ? 420 : 600, cancellation: cancellation))
                            } catch { continuation.resume(throwing: error) }
                        }
                    }
                } onCancel: {
                    cancellation.cancel()
                }
                try Task.checkCancellation()
                guard let self else { return }
                guard self.displayedView === ordered, self.textSize == fontSize else {
                    for ticket in tickets { self.widths.abandon(ticket) }
                    return
                }
                self.applyingWidths = true
                defer { self.applyingWidths = false }
                for ticket in tickets where self.widths.accept(ticket) {
                    if let column = self.table.tableColumn(withIdentifier: self.identifier(for: ticket.column)),
                        let width = result[ticket.column]
                    {
                        column.width = width
                    }
                }
            } catch {
                guard let self else { return }
                for ticket in tickets { self.widths.abandon(ticket) }
                if Task.isCancelled, self.displayedView === ordered { continueFitting = false }
                if !Task.isCancelled, !(error is CancellationError), self.displayedView === ordered {
                    continueFitting = false
                    self.model.reportError(error, prefix: "Could not fit columns: ")
                }
            }
        }
        if automatic { automaticFitTask = task } else { fitTask = task }
    }

    func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
        guard let column = columnId(tableColumn) else { return }
        let multiple = (table.headerView as? FilterHeaderView)?.sortingMultiple ?? false
        DispatchQueue.main.async { [weak self] in
            self?.model.selectedColumn = column
            self?.model.cycleSort(column: column, multiple: multiple)
        }
    }

    private func columnMenu(at index: Int) -> NSMenu? {
        guard index >= 0, index < table.tableColumns.count, let column = columnId(table.tableColumns[index]) else {
            return nil
        }
        let menu = NSMenu()
        let sort = model.draftSort(for: column)
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
            if action == #selector(clearMenuAllSorting(_:)), model.draftSorts.isEmpty { continue }
            if action == #selector(clearMenuColumnFilter(_:)),
                !model.appliedFilterColumns.contains(column)
            {
                continue
            }
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = column
            if action == #selector(sortMenuAscending(_:)), sort?.descending == false { item.state = .on }
            if action == #selector(sortMenuDescending(_:)), sort?.descending == true { item.state = .on }
            menu.addItem(item)
        }
        return menu
    }

    @objc private func analyzeMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { model.inspectColumn(column) }
    }
    @objc private func sortMenuAscending(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId {
            model.setSortDirection(column: column, descending: false)
        }
    }
    @objc private func sortMenuDescending(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { model.setSortDirection(column: column, descending: true) }
    }
    @objc private func fitMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { fitColumns([column], automatic: false) }
    }
    @objc private func fitMenuAll(_ item: NSMenuItem) { fitColumns(displayedColumns, automatic: false) }
    @objc private func customizeMenuColumns(_ item: NSMenuItem) { model.showsColumns = true }

    @objc private func filterMenuColumn(_ item: NSMenuItem) {
        guard let column = item.representedObject as? ColumnId else { return }
        showFilter(at: table.column(withIdentifier: identifier(for: column)))
    }
    @objc private func hideMenuColumn(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { hideColumn(column, in: model) }
    }
    @objc private func clearMenuColumnSort(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { model.removeSort(column: column) }
    }
    @objc private func clearMenuAllSorting(_ item: NSMenuItem) { model.clearSort() }
    @objc private func clearMenuColumnFilter(_ item: NSMenuItem) {
        if let column = item.representedObject as? ColumnId { model.setFilter(nil, column: column) }
    }

    @objc private func tableClicked(_ sender: Any?) { selectColumn(at: table.clickedColumn) }
    private func selectColumn(at index: Int) {
        guard index >= 0, index < table.tableColumns.count, let column = columnId(table.tableColumns[index]) else {
            return
        }
        model.selectedColumn = column
    }

    func scrollToColumn(_ column: ColumnId, focus: Bool = false) {
        let index = table.column(withIdentifier: identifier(for: column))
        guard index >= 0 else { return }
        table.scrollColumnToVisible(index)
        if focus { table.window?.makeFirstResponder(table) }
    }

    func focusTable() { table.window?.makeFirstResponder(table) }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !reloading else { return }
        gutter.needsDisplay = true
    }

    func navigateToMatch(_ match: CellMatch) {
        let row = Int(match.displayRow)
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        table.scrollRowToVisible(row)
        scrollToColumn(match.column)
        requestHighlights()
    }

    func showSelectedColumnFilter() {
        guard let column = model.selectedColumn else { return }
        let index = table.column(withIdentifier: identifier(for: column))
        guard index >= 0 else { return }
        showFilter(at: index)
    }

    func showColumnFilter(_ column: ColumnId) {
        guard model.visibleColumns.contains(column) else { return }
        scrollToColumn(column)
        showFilter(at: table.column(withIdentifier: identifier(for: column)))
    }

    var displayedColumns: [ColumnId] {
        table.tableColumns.filter { !$0.isHidden }.compactMap(columnId)
    }

    private func showFilter(at index: Int) {
        guard let header = table.headerView,
            index >= 0, index < table.tableColumns.count,
            let column = columnId(table.tableColumns[index]),
            let info = model.columnInfo[column]
        else { return }
        closePopover()
        let content = ColumnFilterView(
            viewer: model.viewer, info: info,
            current: model.draftFilter(for: column)
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
        model.copying.copy(
            ordered: model.ordered, rows: Array(table.selectedRowIndexes.rangeView), columns: displayedColumns,
            names: displayedColumns.map { model.columnInfo[$0]?.name ?? "" })
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
