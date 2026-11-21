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
/// Edits change ``TableState``, which keeps columns and query consistent,
/// at once; a changed query runs in the background. `displayed` changes only
/// when a query succeeds, so the table, status bar, Find and analyses always
/// describe one consistent result.
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
    /// Observed per property below, so a view reading one of them updates
    /// only when that one changes.
    @ObservationIgnored private var state: TableState
    private(set) var isUpdating = false

    /// The column whose filter editor is open.
    var filterEditor: ColumnID?
    var showsColumns = false
    var showsSorting = false
    private(set) var reveal: RevealRequest?

    @ObservationIgnored private var queryTask: Task<Void, Never>?
    @ObservationIgnored private var submitted: TableQuery?

    init(url: URL, session: TableSession, alerts: Alerts) {
        self.url = url
        self.session = session
        self.alerts = alerts
        displayed = session.initialRows
        state = TableState(columns: session.source.columns)
        find = FindModel(alerts: alerts)
        copying = CopyModel(alerts: alerts)
        export = ExportModel(alerts: alerts)
        find.onNavigate = { [weak self] match in
            self?.reveal = RevealRequest(row: match.displayRow, column: match.column, focusesTable: false)
        }
        find.update(rows: displayed, columns: visibleColumns)
    }

    /// The query being edited; differs from `displayed.query` while one runs.
    var draft: TableQuery {
        access(keyPath: \.draft)
        return state.query
    }
    var visibleColumns: [ColumnID] {
        access(keyPath: \.visibleColumns)
        return state.visibleColumns
    }
    var selectedColumn: ColumnID? {
        access(keyPath: \.selectedColumn)
        return state.selectedColumn
    }
    /// The column the Inspector analyzes, independent of selection.
    var inspectedColumn: ColumnID? {
        access(keyPath: \.inspectedColumn)
        return state.inspectedColumn
    }
    var chartX: ColumnID? {
        get {
            access(keyPath: \.chartX)
            return state.chartX
        }
        set { edit { $0.setChartX(newValue) } }
    }
    var chartY: ColumnID? {
        get {
            access(keyPath: \.chartY)
            return state.chartY
        }
        set { edit { $0.setChartY(newValue) } }
    }

    /// Change `state`, notifying observers of each property that changed.
    /// Returns the state before the edit.
    @discardableResult
    private func edit(_ change: (inout TableState) -> Void) -> TableState {
        let old = state
        change(&state)
        if state.query != old.query { withMutation(keyPath: \.draft) {} }
        if state.visibleColumns != old.visibleColumns { withMutation(keyPath: \.visibleColumns) {} }
        if state.selectedColumn != old.selectedColumn { withMutation(keyPath: \.selectedColumn) {} }
        if state.inspectedColumn != old.inspectedColumn { withMutation(keyPath: \.inspectedColumn) {} }
        if state.chartX != old.chartX { withMutation(keyPath: \.chartX) {} }
        if state.chartY != old.chartY { withMutation(keyPath: \.chartY) {} }
        return old
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
        edit { $0.select(column) }
    }

    /// Analyze `column` in the Inspector.
    func inspect(_ column: ColumnID) {
        edit { $0.inspect(column) }
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

    // MARK: Query

    func cycleSort(column: ColumnID, multiple: Bool) {
        editQuery { $0.cycleSort(column, multiple: multiple) }
    }

    func setSortDirection(column: ColumnID, descending: Bool) {
        editQuery { $0.setSortDirection(column, descending: descending) }
    }

    func removeSort(column: ColumnID) {
        editQuery { $0.removeSort(column) }
    }

    func clearSorts() {
        editQuery { $0.clearSorts() }
    }

    /// Sort by `replacement` instead of `column`, keeping the rule's priority and direction.
    func replaceSortColumn(_ column: ColumnID, with replacement: ColumnID) {
        editQuery { $0.replaceSortColumn(column, with: replacement) }
    }

    /// Reorder sort priority; `columns` must hold exactly the sorted columns.
    func reorderSorts(_ columns: [ColumnID]) {
        editQuery { $0.reorderSorts(columns) }
    }

    func setFilter(_ filter: Filter?, column: ColumnID) {
        editQuery { $0.setFilter(filter, for: column) }
    }

    func clearFilters() {
        editQuery { $0.clearFilters() }
    }

    var canClearFilters: Bool { !draft.filters.isEmpty }

    /// Apply an edit, running the query when it changed.
    private func editQuery(_ change: (inout TableState) -> Void) {
        if edit(change).query != state.query { applyDraft() }
    }

    // MARK: Columns

    /// Reorder visible columns; `columns` must hold exactly the visible columns.
    func reorderColumns(_ columns: [ColumnID]) {
        guard edit({ $0.reorderColumns(columns) }).visibleColumns != state.visibleColumns else { return }
        // Same rows and the same matches, in a new order: keep the current match.
        if !isUpdating { find.update(rows: displayed, columns: visibleColumns, keepingMatch: true) }
    }

    /// Show or hide columns after any confirmation. Shown columns go to the
    /// end in file order; hiding clears their filters and sorts.
    func setColumnsVisible(_ columns: [ColumnID], visible: Bool) {
        let old = edit { $0.setColumnsVisible(columns, visible: visible) }
        guard state.visibleColumns != old.visibleColumns else { return }
        if let filterEditor, !state.isVisible(filterEditor) { self.filterEditor = nil }
        if state.query != old.query {
            applyDraft()
        } else if !isUpdating {
            find.update(rows: displayed, columns: visibleColumns)
        }
    }

    func showAllColumns() {
        setColumnsVisible(source.columns.map(\.id), visible: true)
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
        copying.cancel()
        find.update(rows: rows, columns: visibleColumns)
    }

    /// The displayed rows stay; the draft returns to them.
    private func fail(_ error: any Error) {
        queryTask = nil
        submitted = nil
        isUpdating = false
        edit { $0.revert(to: displayed.query) }
        alerts.report(error, title: "Could Not Update the Table")
        find.update(rows: displayed, columns: visibleColumns)
    }

    /// Stop all work for this table, before replacing or closing it.
    func stop() {
        queryTask?.cancel()
        queryTask = nil
        submitted = nil
        isUpdating = false
        edit { $0.revert(to: displayed.query) }
        find.stop()
        copying.cancel()
        export.stop()
    }
}

extension TableModel {
    /// Columns with an applied filter.
    var filteredColumns: Set<ColumnID> { Set(appliedQuery.filters.map(\.column)) }
}
