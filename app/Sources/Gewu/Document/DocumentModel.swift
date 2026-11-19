import Foundation
import Observation

enum InspectorPage: String, CaseIterable {
    case column = "Column"
    case chart = "Chart"
}

/// One loaded table's presentation state. Rust owns the data; SwiftUI observes only
/// metadata and immutable views, never an array containing the whole table.
@MainActor
@Observable
final class DocumentModel {
    struct TableInput: Equatable {
        let ordered: GewuOrderedView
        let visibleColumns: [ColumnId]
        let filters: [ColumnFilter]
        let sorts: [ColumnSort]
        let selectedColumn: ColumnId?
        let inspectedColumn: ColumnId?

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.ordered === rhs.ordered
                && lhs.visibleColumns == rhs.visibleColumns
                && lhs.filters == rhs.filters
                && lhs.sorts == rhs.sorts
                && lhs.selectedColumn == rhs.selectedColumn
                && lhs.inspectedColumn == rhs.inspectedColumn
        }
    }

    private struct RowQuery: Equatable {
        let filters: [ColumnFilter]
        let sorts: [ColumnSort]
    }

    let url: URL
    let dataset: GewuDataset
    let info: DatasetInfo
    let columnInfo: [ColumnId: ColumnInfo]
    @ObservationIgnored private let columnOrder: [ColumnId: Int]
    let find: FindModel
    /// The applied filtered/ordered views and cached analysis results.
    let viewer: Viewer
    var filtered: GewuFilteredView { viewer.filtered }
    var ordered: GewuOrderedView { viewer.ordered }

    private(set) var visibleColumns: [ColumnId]
    private(set) var draftVisibleColumns: [ColumnId]
    private(set) var draftFilters: [ColumnFilter]
    private(set) var draftSorts: [ColumnSort]
    private var submitted: RowQuery?
    var isBusy: Bool { submitted != nil }

    private(set) var errorMessage: String?
    var isErrorPresented = false

    @ObservationIgnored private var _selectedColumn: ColumnId?
    var selectedColumn: ColumnId? {
        get {
            access(keyPath: \.selectedColumn)
            return _selectedColumn
        }
        set {
            guard newValue != _selectedColumn else { return }
            withMutation(keyPath: \.selectedColumn) {
                _selectedColumn = newValue
            }
        }
    }

    @ObservationIgnored private var _inspectedColumn: ColumnId?
    var inspectedColumn: ColumnId? {
        get {
            access(keyPath: \.inspectedColumn)
            return _inspectedColumn
        }
        set {
            guard newValue != _inspectedColumn else { return }
            withMutation(keyPath: \.inspectedColumn) {
                _inspectedColumn = newValue
            }
        }
    }

    @ObservationIgnored private var _inspectorVisible = true
    var inspectorVisible: Bool {
        get {
            access(keyPath: \.inspectorVisible)
            return _inspectorVisible
        }
        set {
            guard newValue != _inspectorVisible else { return }
            withMutation(keyPath: \.inspectorVisible) {
                _inspectorVisible = newValue
            }
        }
    }

    var inspectorPage = InspectorPage.column
    var showsSorting = false
    var inspectorRequest = 0
    var showsColumns = false
    let inspectorSettings = InspectorSettings()
    var chartX: ColumnId?
    var chartY: ColumnId?
    var searchFocusRequest = 0

    @ObservationIgnored private(set) lazy var export = ExportModel { [weak self] error, prefix in
        self?.reportError(error, prefix: prefix)
    }
    @ObservationIgnored private(set) lazy var copying = CopyModel { [weak self] error, prefix in
        self?.reportError(error, prefix: prefix)
    }

    @ObservationIgnored weak var table: DataTableController?
    @ObservationIgnored private var viewTask: Task<Void, Never>?

    var tableInput: TableInput {
        TableInput(
            ordered: ordered,
            visibleColumns: visibleColumns,
            filters: viewer.appliedFilters,
            sorts: viewer.appliedSorts,
            selectedColumn: selectedColumn,
            inspectedColumn: inspectorVisible ? inspectedColumn : nil
        )
    }

    /// Receives initial views already allocated by the background loading worker,
    /// or constructs default unfiltered/unsorted views for direct callers and tests.
    init(
        url: URL,
        dataset: GewuDataset,
        filtered: GewuFilteredView? = nil,
        ordered: GewuOrderedView? = nil,
        queryWorker: OperationQueue? = nil
    ) {
        self.url = url
        self.dataset = dataset
        let metadata = dataset.info()
        info = metadata
        columnInfo = Dictionary(uniqueKeysWithValues: metadata.columns.map { ($0.id, $0) })
        columnOrder = Dictionary(uniqueKeysWithValues: metadata.columns.enumerated().map { ($1.id, $0) })
        find = FindModel()
        let initialFiltered = filtered ?? dataset.unfiltered()
        let initialOrdered = ordered ?? initialFiltered.unsorted()
        viewer = Viewer(
            dataset: dataset, filtered: initialFiltered, ordered: initialOrdered, queryWorker: queryWorker)
        let initialColumns = metadata.columns.map(\.id)
        visibleColumns = initialColumns
        draftVisibleColumns = initialColumns
        draftFilters = []
        draftSorts = []
        _selectedColumn = initialColumns.first
        _inspectedColumn = initialColumns.first
        chartY = metadata.columns.first { $0.capabilities.scatter }?.id
        find.onNavigate = { [weak self] match in self?.table?.navigateToMatch(match) }
        find.onError = { [weak self] error, prefix in self?.reportError(error, prefix: prefix) }
    }

    var appliedFilterColumns: Set<ColumnId> {
        Set(viewer.appliedFilters.map(\.column))
    }

    var canClearFilters: Bool {
        !draftFilters.isEmpty || !viewer.appliedFilters.isEmpty
    }

    var canExport: Bool {
        !isBusy && export.isIdle && !visibleColumns.isEmpty
    }

    var subtitle: String {
        let source =
            switch info.metadata {
            case .xlsx(let sheet, _): "\(info.format.displayName) · \(sheet)"
            default: info.format.displayName
            }
        return "\(source) · \(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
            + ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)
    }

    var status: String {
        let rows = ordered.rowCount()
        let text =
            appliedFilterColumns.isEmpty && rows == info.rowCount
            ? "\(formatCount(rows)) rows"
            : "\(formatCount(rows)) / \(formatCount(info.rowCount)) rows"
        return text + " · \(visibleColumns.count) columns"
    }

    func draftFilter(for column: ColumnId) -> Filter? {
        draftFilters.first { $0.column == column }?.filter
    }

    func appliedFilter(for column: ColumnId) -> Filter? {
        viewer.appliedFilters.first { $0.column == column }?.filter
    }

    func draftSort(for column: ColumnId) -> ColumnSort? {
        draftSorts.first { $0.column == column }
    }

    func appliedSort(for column: ColumnId) -> ColumnSort? {
        viewer.appliedSorts.first { $0.column == column }
    }

    func sortPriority(for column: ColumnId, in sorts: [ColumnSort]) -> Int? {
        sorts.firstIndex { $0.column == column }
    }

    func cycleSort(column: ColumnId, multiple: Bool) {
        guard draftVisibleColumns.contains(column) else { return }
        let old = draftSorts.first { $0.column == column }
        var next = multiple ? draftSorts : []
        if old?.descending == true {
            next.removeAll { $0.column == column }
        } else {
            let descending = old != nil
            if let index = next.firstIndex(where: { $0.column == column }) {
                next[index].descending = descending
            } else {
                next.append(ColumnSort(column: column, descending: descending))
            }
        }
        guard next != draftSorts else { return }
        draftSorts = next
        refresh()
    }

    func setSortDirection(column: ColumnId, descending: Bool) {
        guard draftVisibleColumns.contains(column) else { return }
        var next = draftSorts
        if let index = next.firstIndex(where: { $0.column == column }) {
            next[index].descending = descending
        } else {
            next.append(ColumnSort(column: column, descending: descending))
        }
        guard next != draftSorts else { return }
        draftSorts = next
        refresh()
    }

    func removeSort(column: ColumnId) {
        guard draftSorts.contains(where: { $0.column == column }) else { return }
        draftSorts.removeAll { $0.column == column }
        refresh()
    }

    func clearSort() {
        guard !draftSorts.isEmpty else { return }
        draftSorts.removeAll()
        refresh()
    }

    func replaceSortColumn(_ column: ColumnId, with replacement: ColumnId) {
        guard column != replacement,
            draftVisibleColumns.contains(replacement),
            let source = draftSorts.firstIndex(where: { $0.column == column }),
            !draftSorts.contains(where: { $0.column == replacement })
        else { return }
        draftSorts[source] = ColumnSort(column: replacement, descending: draftSorts[source].descending)
        refresh()
    }

    func reorderSort(_ columns: [ColumnId]) {
        guard Set(columns) == Set(draftSorts.map(\.column)), columns.count == draftSorts.count else { return }
        let byColumn = Dictionary(uniqueKeysWithValues: draftSorts.map { ($0.column, $0) })
        let next = columns.compactMap { byColumn[$0] }
        guard next != draftSorts else { return }
        draftSorts = next
        refresh()
    }

    func setFilter(_ filter: Filter?, column: ColumnId) {
        guard draftVisibleColumns.contains(column) else { return }
        var next = draftFilters
        if let filter {
            if let index = next.firstIndex(where: { $0.column == column }) {
                next[index].filter = filter
            } else {
                next.append(ColumnFilter(column: column, filter: filter))
                next.sort { (columnOrder[$0.column] ?? Int.max) < (columnOrder[$1.column] ?? Int.max) }
            }
        } else {
            next.removeAll { $0.column == column }
        }
        guard next != draftFilters else { return }
        draftFilters = next
        refresh()
    }

    func clearFilters() {
        guard canClearFilters else { return }
        guard !draftFilters.isEmpty else { return }
        draftFilters.removeAll()
        refresh()
    }

    func reorderColumns(_ columns: [ColumnId]) {
        guard columns != draftVisibleColumns,
            Set(columns) == Set(draftVisibleColumns),
            columns.count == draftVisibleColumns.count
        else { return }
        draftVisibleColumns = columns
        if !isBusy && draftFilters == viewer.appliedFilters && draftSorts == viewer.appliedSorts {
            visibleColumns = columns
            updateFind(preservingMatch: true)
        } else {
            refresh()
        }
    }

    /// Apply a hide operation after any required confirmation.
    func removeColumn(_ column: ColumnId) {
        setColumnsVisible([column], visible: false)
    }

    func showColumn(_ column: ColumnId) {
        setColumnsVisible([column], visible: true)
    }

    func showAllColumns() {
        setColumnsVisible(info.columns.map(\.id), visible: true)
    }

    /// Applies a confirmed bulk visibility edit, preserving surviving display order.
    func setColumnsVisible(_ columns: [ColumnId], visible: Bool) {
        let targets = Set(columns.filter { columnInfo[$0] != nil })
        guard !targets.isEmpty else { return }
        if visible {
            let shown = Set(draftVisibleColumns)
            let additions = info.columns.map(\.id).filter { targets.contains($0) && !shown.contains($0) }
            guard !additions.isEmpty else { return }
            draftVisibleColumns += additions
        } else {
            let nextColumns = draftVisibleColumns.filter { !targets.contains($0) }
            let nextFilters = draftFilters.filter { !targets.contains($0.column) }
            let nextSorts = draftSorts.filter { !targets.contains($0.column) }
            guard nextColumns != draftVisibleColumns || nextFilters != draftFilters || nextSorts != draftSorts
            else { return }
            draftVisibleColumns = nextColumns
            draftFilters = nextFilters
            draftSorts = nextSorts
        }
        if !isBusy && draftFilters == viewer.appliedFilters && draftSorts == viewer.appliedSorts {
            visibleColumns = draftVisibleColumns
            reconcileVisibleColumnSelections()
            updateFind()
        } else {
            refresh()
        }
    }

    private func isPlottable(_ column: ColumnId) -> Bool {
        columnInfo[column]?.capabilities.scatter == true
    }

    private func reconcileVisibleColumnSelections() {
        let visible = Set(visibleColumns)
        if selectedColumn.map({ !visible.contains($0) }) ?? true {
            selectedColumn = visibleColumns.first
        }
        if inspectedColumn.map({ !visible.contains($0) }) ?? true {
            inspectedColumn = visibleColumns.first
        }
        if let chartX, !(visible.contains(chartX) && isPlottable(chartX)) {
            self.chartX = nil
        }
        if chartY.map({ !(visible.contains($0) && isPlottable($0)) }) ?? true {
            self.chartY = visibleColumns.first { isPlottable($0) }
        }
    }

    func inspectColumn(_ column: ColumnId) {
        guard columnInfo[column] != nil, visibleColumns.contains(column) else { return }
        inspectedColumn = column
        inspectorPage = .column
        inspectorRequest &+= 1
    }

    func refresh(debounce: Bool = false) {
        let target = RowQuery(filters: draftFilters, sorts: draftSorts)
        guard target != submitted else { return }
        viewTask?.cancel()
        let applied = RowQuery(filters: viewer.appliedFilters, sorts: viewer.appliedSorts)
        if target == applied {
            viewTask = nil
            let wasBusy = submitted != nil
            submitted = nil
            let columnsChanged = visibleColumns != draftVisibleColumns
            if columnsChanged {
                visibleColumns = draftVisibleColumns
                reconcileVisibleColumnSelections()
            }
            if wasBusy || columnsChanged {
                updateFind()
            }
            return
        }
        let filtersChanged = target.filters != viewer.appliedFilters
        find.invalidate()
        submitted = target
        let viewer = viewer
        let currentFiltered = viewer.filtered
        viewTask = Task { [weak self] in
            do {
                let nextFiltered: GewuFilteredView?
                let nextOrdered: GewuOrderedView
                if filtersChanged {
                    let filteredView = try await viewer.filter(target.filters, debounce: debounce)
                    try Task.checkCancellation()
                    nextOrdered = try await viewer.sort(target.sorts, on: filteredView, debounce: false)
                    nextFiltered = filteredView
                } else {
                    nextFiltered = nil
                    nextOrdered = try await viewer.sort(target.sorts, on: currentFiltered, debounce: debounce)
                }
                try Task.checkCancellation()
                guard let self else { return }
                var prepared: TableSegmentLoader.Snapshot?
                while let table = self.table {
                    let columns = self.draftVisibleColumns
                    let snapshot = try await table.loadViewport(in: nextOrdered, columns: columns)
                    try Task.checkCancellation()
                    if self.table === table && self.draftVisibleColumns == columns {
                        prepared = snapshot
                        break
                    }
                }
                try Task.checkCancellation()
                self.commit(
                    target: target,
                    visibleColumns: self.draftVisibleColumns,
                    filtered: nextFiltered,
                    ordered: nextOrdered,
                    rows: prepared
                )
            } catch {
                guard !Task.isCancelled, let self else { return }
                self.viewTask = nil
                self.submitted = nil
                self.draftFilters = self.viewer.appliedFilters
                self.draftSorts = self.viewer.appliedSorts
                let columnsReverted = self.draftVisibleColumns != self.visibleColumns
                self.draftVisibleColumns = self.visibleColumns
                if columnsReverted { self.table?.syncColumns() }
                if !error.isCancellation { self.reportError(error) }
                self.updateFind()
            }
        }
    }

    /// Publish the resolved views and ready cells together, without suspending or
    /// waiting for SwiftUI to deliver another table update.
    private func commit(
        target: RowQuery,
        visibleColumns: [ColumnId],
        filtered: GewuFilteredView?,
        ordered: GewuOrderedView,
        rows: TableSegmentLoader.Snapshot?
    ) {
        if let filtered {
            viewer.apply(filtered: filtered, ordered: ordered, filters: target.filters, sorts: target.sorts)
        } else {
            viewer.apply(ordered: ordered, sorts: target.sorts)
        }
        self.visibleColumns = visibleColumns
        reconcileVisibleColumnSelections()
        if let rows { table?.commit(rows) }
        copying.cancel()
        viewTask = nil
        submitted = nil
        updateFind()
    }

    func updateFind(debounce: Bool = false, preservingMatch: Bool = false) {
        guard !isBusy else {
            find.invalidate()
            return
        }
        find.update(
            ordered: ordered, columns: visibleColumns,
            cursor: preservingMatch ? find.currentMatch : nil, debounce: debounce)
    }

    func stop() {
        viewTask?.cancel()
        viewTask = nil
        submitted = nil
        viewer.close()
        draftFilters = viewer.appliedFilters
        draftSorts = viewer.appliedSorts
        let columnsReverted = draftVisibleColumns != visibleColumns
        draftVisibleColumns = visibleColumns
        if columnsReverted { table?.syncColumns() }
        find.stop()
        export.stop()
        copying.cancel()
        table?.stopLoading()
    }

    func findNext(backwards: Bool = false) {
        find.next(backwards: backwards)
    }

    func focusSearch() {
        searchFocusRequest &+= 1
    }

    private func reportError(_ message: String) {
        errorMessage = message
        isErrorPresented = true
    }

    func reportError(_ error: Error, prefix: String = "") {
        reportError(prefix + error.userMessage)
    }
}
