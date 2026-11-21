import AppKit
import GewuKit
import SwiftUI

/// Hosts the native table. It renders a ``TablePresentation`` and turns
/// table gestures into ``TableModel`` intents; the model never calls back.
///
/// When the presentation names new rows, the table keeps showing its current
/// rows while it reads the new viewport, then switches rows, headers and
/// cells in one main-actor turn, so a query never flashes empty cells.
@MainActor
final class TableViewController: NSViewController {
    let model: TableModel
    /// Show a column's analysis; the window owns the Inspector.
    let analyze: (ColumnID) -> Void
    private(set) var presentation: TablePresentation
    /// The rows on screen. Trails `presentation.rows` while new rows are read.
    private(set) var shownRows: DisplayRows

    let table = ContextTableView()
    let scroll = TableScrollView()
    private(set) lazy var gutter = RowGutterView(table: table)
    let texts: TableTextCache
    var widths = ColumnWidths()
    var fonts = CellFonts()

    var columnsByIdentifier: [NSUserInterfaceItemIdentifier: ColumnID] = [:]
    /// The viewport whose cells were last bound, to skip rebinding them.
    var boundViewport = Viewport.empty
    var applyingColumns = false
    var applyingWidths = false
    var switchingRows = false
    var fitTask: Task<Void, Never>?
    var automaticFitTask: Task<Void, Never>?
    var fittingAutomatically = false
    var firstWidthsPending = true
    var filterPopover: (column: ColumnID, popover: NSPopover)?
    var cellPopover: NSPopover?

    private var switching: (rows: DisplayRows, task: Task<Void, Never>)?
    private var handledReveal: UUID?
    private var windowObservers: [any NSObjectProtocol] = []
    let rowHeaderID = NSUserInterfaceItemIdentifier("rowHeader")
    let rowViewID = NSUserInterfaceItemIdentifier("stripedRow")

    init(
        model: TableModel, presentation: TablePresentation, texts: TableTextCache = TableTextCache(),
        analyze: @escaping (ColumnID) -> Void = { _ in }
    ) {
        self.model = model
        self.analyze = analyze
        self.presentation = presentation
        shownRows = presentation.rows
        self.texts = texts
        super.init(nibName: nil, bundle: nil)
        texts.onLoad = { [weak self] blocks in self?.display(blocks) }
        texts.onError = { [weak self] error in self?.model.reportError(error, title: "Could Not Load Rows") }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    // MARK: View lifecycle

    override func loadView() {
        let root = NSView()
        let header = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: 32))
        header.onFilterClick = { [weak self] index in self?.editFilter(at: index) }
        header.columnMenu = { [weak self] index in self?.columnMenu(at: index) }
        table.headerView = header
        table.alphaValue = 0
        table.target = self
        table.action = #selector(tableClicked(_:))
        table.doubleAction = #selector(viewClickedCell(_:))
        table.cellMenu = { [weak self] row, column in self?.cellMenu(row: row, index: column) }
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
        scroll.onScroll = { [weak self] in self?.visibleRectChanged() }
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
        buildColumns()
        texts.reset(to: shownRows)
        applyStyle(presentation.style, previous: nil)
        applyColumnLayout(presentation.columns)
        refreshHeaders(restyle: true)
        table.noteNumberOfRowsChanged()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        guard let window = view.window, windowObservers.isEmpty else { return }
        // Selected-row text colors depend on whether this window is key.
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            windowObservers.append(
                NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.refreshCellAppearance() }
                })
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        updateViewport()
        fitVisibleColumns()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        windowObservers.forEach(NotificationCenter.default.removeObserver)
        windowObservers.removeAll()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        updateViewport()
        fitVisibleColumns()
    }

    /// Stop reads, fits and popovers before the table goes away.
    func stop() {
        switching?.task.cancel()
        switching = nil
        texts.stop()
        stopFitting()
        closeFilterEditor()
        cellPopover?.close()
    }

    // MARK: Presentation

    func present(_ next: TablePresentation) {
        loadViewIfNeeded()
        let previous = presentation
        presentation = next
        let restyle = applyStyle(next.style, previous: previous.style)
        if next.columns != previous.columns {
            applyColumnLayout(next.columns)
            table.layoutSubtreeIfNeeded()
        }
        if next.rows === shownRows {
            switching?.task.cancel()
            switching = nil
        } else if switching?.rows !== next.rows {
            beginSwitch(to: next.rows)
        }
        if restyle || next.selectedColumn != previous.selectedColumn
            || next.inspectedColumn != previous.inspectedColumn
        {
            refreshHeaders(restyle: restyle)
        }
        if restyle || next.find != previous.find {
            refreshVisibleCells(restyle: restyle)
        }
        presentFilterEditor(next.filterEditor)
        performReveal()
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    /// Returns whether cells need restyling.
    @discardableResult
    private func applyStyle(_ style: TablePresentation.Style, previous: TablePresentation.Style?) -> Bool {
        let height = max(style.rowHeight, style.fontSize + 8)
        let restyle =
            previous == nil || previous?.fontSize != style.fontSize || previous?.rowHeight != style.rowHeight
            || previous?.rowNumberBase != style.rowNumberBase
        if restyle {
            if previous != nil, previous?.fontSize != style.fontSize { widths.fontChanged() }
            fonts = CellFonts(size: style.fontSize)
            table.rowHeight = height
            table.headerView?.frame.size.height = max(32, ceil(style.fontSize * 2.6))
            scroll.tile()
            gutter.font = fonts.rowNumber
            fitGutter()
            gutter.needsDisplay = true
        }
        if previous?.showsRowNumbers != style.showsRowNumbers {
            gutter.setShown(style.showsRowNumbers)
            table.tableColumn(withIdentifier: rowHeaderID)?.isHidden = !style.showsRowNumbers
        }
        return restyle
    }

    // MARK: Switching rows

    private func beginSwitch(to rows: DisplayRows) {
        switching?.task.cancel()
        let task = Task {
            let prepared: PreparedText
            do {
                prepared = try await texts.prepare(rows) { predictedViewport(for: rows) }
            } catch {
                return
            }
            guard switching?.rows === rows else { return }
            switching = nil
            commit(prepared)
        }
        switching = (rows, task)
    }

    /// Install prepared rows and fill visible cells in one turn.
    private func commit(_ prepared: PreparedText) {
        stopFitting()
        let rowCountChanged = shownRows.count != prepared.rows.count
        switchingRows = true
        shownRows = prepared.rows
        texts.install(prepared)
        table.deselectAll(nil)
        if rowCountChanged {
            table.noteNumberOfRowsChanged()
            table.layoutSubtreeIfNeeded()
        }
        // Cells bound to the old rows rebind as they are refreshed or enter the viewport.
        boundViewport = .empty
        refreshVisibleCells(restyle: false)
        refreshHeaders(restyle: false)
        gutter.needsDisplay = true
        switchingRows = false
        performReveal()
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    /// The viewport of `rows` after their row count takes effect, with one
    /// bordering row for fractional scrolling and clipping.
    private func predictedViewport(for rows: DisplayRows) -> Viewport {
        let count = rows.count
        let clip = scroll.contentView.bounds
        let stride = table.rowHeight + table.intercellSpacing.height
        let y = max(0, min(clip.minY, CGFloat(count) * stride - clip.height))
        let first = min(count, max(0, Int(floor(y / stride)) - 1))
        let last = min(count, Int(ceil((y + clip.height) / stride)) + 1)
        return Viewport(rows: first..<max(first, last), columns: visibleColumns(in: clip))
    }

    // MARK: Reveal

    private func performReveal() {
        guard let reveal = presentation.reveal, reveal.id != handledReveal else { return }
        // A match in rows not shown yet is revealed once they are.
        guard reveal.row == nil || presentation.rows === shownRows else { return }
        handledReveal = reveal.id
        if let row = reveal.row, row < shownRows.count {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            table.scrollRowToVisible(row)
        }
        if let column = reveal.column {
            let index = table.column(withIdentifier: identifier(for: column))
            if index >= 0 { table.scrollColumnToVisible(index) }
        }
        if reveal.focusesTable { view.window?.makeFirstResponder(table) }
    }

    // MARK: Columns

    func identifier(for id: ColumnID) -> NSUserInterfaceItemIdentifier {
        NSUserInterfaceItemIdentifier("c\(id.raw)")
    }

    func columnID(_ column: NSTableColumn) -> ColumnID? {
        columnsByIdentifier[column.identifier]
    }

    func columnID(at index: Int) -> ColumnID? {
        guard index >= 0, index < table.tableColumns.count else { return nil }
        return columnID(table.tableColumns[index])
    }

    /// Visible data columns in display order.
    var displayedColumns: [ColumnID] {
        table.tableColumns.filter { !$0.isHidden }.compactMap(columnID)
    }

    func visibleColumns(in rect: NSRect) -> [ColumnID] {
        table.columnIndexes(in: rect).compactMap { index in
            let column = table.tableColumns[index]
            return column.isHidden ? nil : columnID(column)
        }
    }

    private func buildColumns() {
        gutter.labels = { [weak self] rows in self?.rowNumberLabels(rows) ?? [] }
        let rowHeader = NSTableColumn(identifier: rowHeaderID)
        rowHeader.headerCell = ColumnHeaderCell(textCell: "")
        rowHeader.resizingMask = []
        table.addTableColumn(rowHeader)
        for column in model.source.columns {
            let identifier = identifier(for: column.id)
            columnsByIdentifier[identifier] = column.id
            let tableColumn = NSTableColumn(identifier: identifier)
            let header = ColumnHeaderCell(textCell: column.name)
            header.subtitle = column.displayType
            header.showsFunnel = true
            tableColumn.headerCell = header
            tableColumn.width = 140
            tableColumn.minWidth = 50
            table.addTableColumn(tableColumn)
        }
    }

    /// Source row numbers of the shown rows, read synchronously when the gutter draws.
    func rowNumberLabels(_ rows: Range<Int>) -> [String] {
        let base = presentation.style.rowNumberBase
        return shownRows.rowNumbers(rows).map { formatRowNumber($0, base: base) }
    }

    private func fitGutter() {
        let base = presentation.style.rowNumberBase
        gutter.fit(largest: rowNumber(max(model.info.rowCount, 1) - 1, base: base))
        gutter.toolTip = "Row in file (\(formatRowNumber(0, base: base))-based)"
        if let column = table.tableColumn(withIdentifier: rowHeaderID) {
            column.minWidth = gutter.fittedWidth
            column.maxWidth = gutter.fittedWidth
            column.width = gutter.fittedWidth
        }
    }

    private func applyColumnLayout(_ visible: [ColumnID]) {
        applyingColumns = true
        defer { applyingColumns = false }
        let shown = Set(visible)
        for column in table.tableColumns {
            if let id = columnID(column) { column.isHidden = !shown.contains(id) }
        }
        for (offset, id) in visible.enumerated() {
            let from = table.column(withIdentifier: identifier(for: id))
            if from != offset + 1 { table.moveColumn(from, toColumn: offset + 1) }
        }
    }

    /// Header state follows the shown rows, so it changes with the cells.
    func refreshHeaders(restyle: Bool) {
        let query = shownRows.query
        let filters = Dictionary(uniqueKeysWithValues: query.filters.map { ($0.column, $0.filter) })
        let priorities = Dictionary(uniqueKeysWithValues: query.sorts.enumerated().map { ($1.column, $0) })
        for column in table.tableColumns {
            guard let id = columnID(column), let info = model.column(id),
                let header = column.headerCell as? ColumnHeaderCell
            else { continue }
            let filter = filters[id]
            let sort = query.sort(for: id)
            header.textSize = presentation.style.fontSize
            header.filtered = filter != nil
            header.inspected = presentation.inspectedColumn == id
            header.active = presentation.selectedColumn == id
            header.sorted = sort != nil
            header.sortAscending = !(sort?.descending ?? false)
            header.sortPriority = query.sorts.count > 1 ? (priorities[id].map { $0 + 1 } ?? 0) : 0
            column.headerToolTip =
                "\(info.name): \(info.technicalType)"
                + (filter.map { "\n\($0.detailSummary)" } ?? "")
                + "\nClick to cycle sorting; Shift-click for multiple keys. Use the funnel to filter."
                + "\nDrag to reorder; double-click a divider to fit the column."
        }
        table.headerView?.needsDisplay = true
    }

    // MARK: Viewport

    private func visibleRectChanged() {
        gutter.needsDisplay = true
        updateViewport()
        requestHighlights()
        fitVisibleColumns()
    }

    func updateViewport() {
        guard !switchingRows, !applyingColumns else { return }
        let visible = table.rows(in: table.visibleRect)
        guard visible.location != NSNotFound else {
            texts.updateViewport(.empty)
            return
        }
        let viewport = Viewport(
            rows: visible.location..<(visible.location + visible.length), columns: visibleColumns(in: table.visibleRect)
        )
        if viewport != boundViewport {
            refreshVisibleCells(restyle: true, skipping: boundViewport)
            boundViewport = viewport
        }
        texts.updateViewport(viewport)
    }

    /// Request match flags only for the rows and columns on screen.
    func requestHighlights() {
        guard presentation.rows === shownRows else { return }
        let rows = table.rows(in: table.visibleRect)
        guard rows.location != NSNotFound, rows.length > 0 else { return }
        model.find.requestHighlights(
            rows: rows.location..<(rows.location + rows.length), columns: visibleColumns(in: table.visibleRect))
    }

    // MARK: Actions

    @objc private func tableClicked(_ sender: Any?) {
        if let column = columnID(at: table.clickedColumn) { model.select(column) }
    }

    @objc func copy(_ sender: Any?) {
        guard !table.selectedRowIndexes.isEmpty else { return }
        model.copying.copy(
            from: shownRows, rows: Array(table.selectedRowIndexes.rangeView), columns: displayedColumns)
    }
}

/// A scroll view that reports every change of its visible area.
final class TableScrollView: NSScrollView {
    var onScroll: (() -> Void)?

    override func reflectScrolledClipView(_ clipView: NSClipView) {
        super.reflectScrolledClipView(clipView)
        onScroll?()
    }
}

/// A table that asks for a menu for the clicked cell.
final class ContextTableView: NSTableView {
    var cellMenu: ((Int, Int) -> NSMenu?)?

    override func menu(for event: NSEvent) -> NSMenu? {
        let point = convert(event.locationInWindow, from: nil)
        return cellMenu?(row(at: point), column(at: point))
    }
}
