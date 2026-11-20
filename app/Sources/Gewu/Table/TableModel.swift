import Foundation
import GewuKit
import Observation

/// A cell or column the table should scroll to. Each request has its own
/// identity, so asking for the same place twice scrolls twice.
struct RevealRequest: Equatable {
    let id = UUID()
    let row: Int?
    let column: ColumnID?
    let focusesTable: Bool
}

/// One opened table: which rows it shows, which columns, and what the user
/// is editing. Rust owns the data; this model holds immutable query results
/// from GewuKit and never refers to the views that present it.
///
/// Query edits change `draft` at once and run in the background. `displayed`
/// changes only when a query succeeds, so the table, status bar, Find and
/// analyses always describe one consistent result. Column visibility is
/// presentation and applies immediately; hiding a column removes its
/// conditions from the draft.
@MainActor
@Observable
final class TableModel {
    let url: URL
    let session: TableSession
    let find: FindModel
    let copying: CopyModel
    let export: ExportModel
    @ObservationIgnored private let alerts: Alerts

    /// The rows the table shows. Their query is the applied query.
    private(set) var displayed: DisplayRows
    /// The query being edited; differs from `displayed.query` while one runs.
    private(set) var draft = TableQuery()
    private(set) var visibleColumns: [ColumnID]
    private(set) var isUpdating = false

    private(set) var selectedColumn: ColumnID?
    /// The column the Inspector analyzes, independent of selection.
    private(set) var inspectedColumn: ColumnID?
    var chartX: ColumnID?
    var chartY: ColumnID?

    /// The column whose filter editor is open.
    var filterEditor: ColumnID?
    var showsColumns = false
    var showsSorting = false
    private(set) var reveal: RevealRequest?

    @ObservationIgnored private var queryTask: Task<Void, Never>?
    @ObservationIgnored private var submitted: TableQuery?
    @ObservationIgnored private let columnOrder: [ColumnID: Int]

    init(url: URL, session: TableSession, alerts: Alerts) {
        self.url = url
        self.session = session
        self.alerts = alerts
        displayed = session.initialRows
        let columns = session.source.columns
        visibleColumns = columns.map(\.id)
        columnOrder = Dictionary(uniqueKeysWithValues: columns.enumerated().map { ($1.id, $0) })
        selectedColumn = columns.first?.id
        inspectedColumn = columns.first?.id
        chartY = columns.first { $0.capabilities.scatter }?.id
        find = FindModel(alerts: alerts)
        copying = CopyModel(alerts: alerts)
        export = ExportModel(alerts: alerts)
        find.onNavigate = { [weak self] match in
            self?.reveal = RevealRequest(row: match.displayRow, column: match.column, focusesTable: false)
        }
        find.update(rows: displayed, columns: visibleColumns)
    }

    var source: TableSource { session.source }
    var info: TableInfo { source.info }
    var appliedQuery: TableQuery { displayed.query }

    func column(_ id: ColumnID) -> ColumnInfo? { source.column(id) }

    /// Report a failure from presenting this table, such as a row read.
    func reportError(_ error: any Error, title: String) {
        alerts.report(error, title: title)
    }

    var canExport: Bool { !isUpdating && export.isIdle && !visibleColumns.isEmpty }

    // MARK: Selection and navigation

    func select(_ column: ColumnID?) {
        guard selectedColumn != column else { return }
        selectedColumn = column
    }

    /// Analyze `column` in the Inspector.
    func inspect(_ column: ColumnID) {
        guard visibleColumns.contains(column), inspectedColumn != column else { return }
        inspectedColumn = column
    }

    func revealColumn(_ column: ColumnID, focusingTable: Bool = false) {
        reveal = RevealRequest(row: nil, column: column, focusesTable: focusingTable)
    }

    func focusTable() {
        reveal = RevealRequest(row: nil, column: nil, focusesTable: true)
    }

    func editFilter(_ column: ColumnID) {
        guard visibleColumns.contains(column) else { return }
        filterEditor = column
    }

    // MARK: Sorting

    func cycleSort(column: ColumnID, multiple: Bool) {
        guard visibleColumns.contains(column) else { return }
        let old = draft.sort(for: column)
        var sorts = multiple ? draft.sorts : []
        if old?.descending == true {
            sorts.removeAll { $0.column == column }
        } else if let index = sorts.firstIndex(where: { $0.column == column }) {
            sorts[index].descending = true
        } else {
            sorts.append(ColumnSort(column: column, descending: old != nil))
        }
        editSorts(sorts)
    }

    func setSortDirection(column: ColumnID, descending: Bool) {
        guard visibleColumns.contains(column) else { return }
        var sorts = draft.sorts
        if let index = sorts.firstIndex(where: { $0.column == column }) {
            sorts[index].descending = descending
        } else {
            sorts.append(ColumnSort(column: column, descending: descending))
        }
        editSorts(sorts)
    }

    func removeSort(column: ColumnID) {
        editSorts(draft.sorts.filter { $0.column != column })
    }

    func clearSorts() {
        editSorts([])
    }

    /// Sort by `replacement` instead of `column`, keeping the rule's priority and direction.
    func replaceSortColumn(_ column: ColumnID, with replacement: ColumnID) {
        guard visibleColumns.contains(replacement), draft.sort(for: replacement) == nil,
            let index = draft.sorts.firstIndex(where: { $0.column == column })
        else { return }
        var sorts = draft.sorts
        sorts[index].column = replacement
        editSorts(sorts)
    }

    /// Reorder sort priority; `columns` must hold exactly the sorted columns.
    func reorderSorts(_ columns: [ColumnID]) {
        guard columns.count == draft.sorts.count, Set(columns) == Set(draft.sorts.map(\.column)) else { return }
        editSorts(columns.compactMap(draft.sort(for:)))
    }

    private func editSorts(_ sorts: [ColumnSort]) {
        guard sorts != draft.sorts else { return }
        draft.sorts = sorts
        applyDraft()
    }

    // MARK: Filtering

    func setFilter(_ filter: Filter?, column: ColumnID) {
        guard visibleColumns.contains(column) else { return }
        var filters = draft.filters
        if let filter {
            if let index = filters.firstIndex(where: { $0.column == column }) {
                filters[index].filter = filter
            } else {
                filters.append(ColumnFilter(column: column, filter: filter))
                // Keep a stable order, so equal conditions compare equal.
                filters.sort { (columnOrder[$0.column] ?? .max) < (columnOrder[$1.column] ?? .max) }
            }
        } else {
            filters.removeAll { $0.column == column }
        }
        guard filters != draft.filters else { return }
        draft.filters = filters
        applyDraft()
    }

    func clearFilters() {
        guard !draft.filters.isEmpty else { return }
        draft.filters = []
        applyDraft()
    }

    var canClearFilters: Bool { !draft.filters.isEmpty }

    // MARK: Columns

    /// Reorder visible columns; `columns` must hold exactly the visible columns.
    func reorderColumns(_ columns: [ColumnID]) {
        guard columns != visibleColumns, columns.count == visibleColumns.count,
            Set(columns) == Set(visibleColumns)
        else { return }
        visibleColumns = columns
        // Same rows and the same matches, in a new order: keep the current match.
        if !isUpdating { find.update(rows: displayed, columns: columns, keepingMatch: true) }
    }

    /// Show or hide columns after any confirmation. Shown columns go to the
    /// end in file order; hiding clears their filters and sorts.
    func setColumnsVisible(_ columns: [ColumnID], visible: Bool) {
        let targets = Set(columns.filter { source.column($0) != nil })
        if visible {
            let shown = Set(visibleColumns)
            let added = source.columns.map(\.id).filter { targets.contains($0) && !shown.contains($0) }
            guard !added.isEmpty else { return }
            visibleColumns += added
        } else {
            let remaining = visibleColumns.filter { !targets.contains($0) }
            guard remaining != visibleColumns else { return }
            visibleColumns = remaining
        }
        reconcileColumnSelections()
        var query = draft
        query.filters.removeAll { !visibleColumns.contains($0.column) }
        query.sorts.removeAll { !visibleColumns.contains($0.column) }
        if query != draft {
            draft = query
            applyDraft()
        } else if !isUpdating {
            find.update(rows: displayed, columns: visibleColumns)
        }
    }

    func showAllColumns() {
        setColumnsVisible(source.columns.map(\.id), visible: true)
    }

    /// Selection, analysis and chart axes fall back to visible columns.
    private func reconcileColumnSelections() {
        let visible = Set(visibleColumns)
        func plottable(_ column: ColumnID) -> Bool {
            visible.contains(column) && source.column(column)?.capabilities.scatter == true
        }
        if selectedColumn.map({ !visible.contains($0) }) ?? true { selectedColumn = visibleColumns.first }
        if inspectedColumn.map({ !visible.contains($0) }) ?? true { inspectedColumn = visibleColumns.first }
        if let chartX, !plottable(chartX) { self.chartX = nil }
        if chartY.map({ !plottable($0) }) ?? true { chartY = visibleColumns.first(where: plottable) }
        if let filterEditor, !visible.contains(filterEditor) { self.filterEditor = nil }
    }

    // MARK: Running queries

    /// Run the draft query unless it is already displayed or running. A newer
    /// draft replaces a running query; returning to the displayed query cancels it.
    private func applyDraft() {
        let target = draft
        guard target != submitted else { return }
        queryTask?.cancel()
        queryTask = nil
        guard target != displayed.query else {
            if isUpdating {
                submitted = nil
                isUpdating = false
                find.update(rows: displayed, columns: visibleColumns)
            }
            return
        }
        submitted = target
        isUpdating = true
        // Matches of the old rows would point at the wrong cells once the new ones show.
        find.invalidate()
        let current = displayed
        queryTask = Task { [session] in
            do {
                let rows = try await session.apply(target, after: current)
                try Task.checkCancellation()
                self.show(rows)
            } catch is CancellationError {
                // Replaced by a newer draft, or the table closed.
            } catch {
                self.fail(error)
            }
        }
    }

    private func show(_ rows: DisplayRows) {
        queryTask = nil
        submitted = nil
        isUpdating = false
        displayed = rows
        reconcileColumnSelections()
        copying.cancel()
        find.update(rows: rows, columns: visibleColumns)
    }

    /// The displayed rows stay; the draft returns to them.
    private func fail(_ error: any Error) {
        queryTask = nil
        submitted = nil
        isUpdating = false
        draft = displayed.query
        alerts.report(error, title: "Could Not Update the Table")
        find.update(rows: displayed, columns: visibleColumns)
    }

    /// Stop all work for this table, before replacing or closing it.
    func stop() {
        queryTask?.cancel()
        queryTask = nil
        submitted = nil
        isUpdating = false
        draft = displayed.query
        find.stop()
        copying.cancel()
        export.stop()
    }
}

extension TableModel {
    /// Columns with an applied filter.
    var filteredColumns: Set<ColumnID> { Set(appliedQuery.filters.map(\.column)) }

    func copySelection(rows: [Range<Int>]) {
        copying.copy(from: displayed, rows: rows, columns: visibleColumns)
    }
}
