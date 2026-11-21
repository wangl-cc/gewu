/// What the user has set up for one table: the query being edited, which
/// columns are visible and in what order, and which columns the selection,
/// the Inspector and the chart use.
///
/// A value with the rules that keep these consistent: every edit names only
/// visible columns, filters stay in schema order so equal queries compare
/// equal, hiding a column removes its filter and sort, and the column
/// choices fall back to visible columns. Running the query and showing its
/// result belong to the caller.
public struct TableState: Equatable, Sendable {
    /// The query being edited.
    public private(set) var query = TableQuery()
    public private(set) var visibleColumns: [ColumnID]
    public private(set) var selectedColumn: ColumnID?
    /// The column the Inspector analyzes, independent of selection.
    public private(set) var inspectedColumn: ColumnID?
    /// The scatter plot's x axis; `nil` plots against the source row number.
    public private(set) var chartX: ColumnID?
    public private(set) var chartY: ColumnID?

    private let schema: [ColumnID]
    private let order: [ColumnID: Int]
    private let plottable: Set<ColumnID>

    /// Every column visible in file order, with the first column selected and
    /// inspected and the first plottable column on the chart's y axis.
    public init(columns: [ColumnInfo]) {
        schema = columns.map(\.id)
        order = Dictionary(uniqueKeysWithValues: schema.enumerated().map { ($1, $0) })
        plottable = Set(columns.filter(\.capabilities.scatter).map(\.id))
        visibleColumns = schema
        selectedColumn = schema.first
        inspectedColumn = schema.first
        chartY = schema.first(where: plottable.contains)
    }

    public func isVisible(_ column: ColumnID) -> Bool { visibleColumns.contains(column) }

    // MARK: Column choices

    public mutating func select(_ column: ColumnID?) {
        guard column.map(isVisible) ?? true else { return }
        selectedColumn = column
    }

    public mutating func inspect(_ column: ColumnID) {
        guard isVisible(column) else { return }
        inspectedColumn = column
    }

    /// `nil` plots against the source row number.
    public mutating func setChartX(_ column: ColumnID?) {
        guard column.map(canPlot) ?? true else { return }
        chartX = column
    }

    public mutating func setChartY(_ column: ColumnID?) {
        guard column.map(canPlot) ?? true else { return }
        chartY = column
    }

    private func canPlot(_ column: ColumnID) -> Bool { isVisible(column) && plottable.contains(column) }

    // MARK: Sorting

    /// A plain click sorts by `column` alone; with `multiple`, the other keys
    /// stay. Each click moves the column from unsorted to ascending to
    /// descending and back to unsorted; a new key goes last.
    public mutating func cycleSort(_ column: ColumnID, multiple: Bool) {
        guard isVisible(column) else { return }
        let old = query.sort(for: column)
        var sorts = multiple ? query.sorts : []
        if old?.descending == true {
            sorts.removeAll { $0.column == column }
        } else if let index = sorts.firstIndex(where: { $0.column == column }) {
            sorts[index].descending = true
        } else {
            sorts.append(ColumnSort(column: column, descending: old != nil))
        }
        query.sorts = sorts
    }

    /// Sort `column` in a direction, keeping its priority or adding it last.
    public mutating func setSortDirection(_ column: ColumnID, descending: Bool) {
        guard isVisible(column) else { return }
        if let index = query.sorts.firstIndex(where: { $0.column == column }) {
            query.sorts[index].descending = descending
        } else {
            query.sorts.append(ColumnSort(column: column, descending: descending))
        }
    }

    public mutating func removeSort(_ column: ColumnID) {
        query.sorts.removeAll { $0.column == column }
    }

    public mutating func clearSorts() {
        query.sorts = []
    }

    /// Sort by `replacement` instead of `column`, keeping the key's priority and direction.
    public mutating func replaceSortColumn(_ column: ColumnID, with replacement: ColumnID) {
        guard isVisible(replacement), query.sort(for: replacement) == nil,
            let index = query.sorts.firstIndex(where: { $0.column == column })
        else { return }
        query.sorts[index].column = replacement
    }

    /// Reorder sort priority; `columns` must hold exactly the sorted columns.
    public mutating func reorderSorts(_ columns: [ColumnID]) {
        guard columns.count == query.sorts.count, Set(columns) == Set(query.sorts.map(\.column)) else { return }
        query.sorts = columns.compactMap(query.sort(for:))
    }

    // MARK: Filtering

    /// Set or, with `nil`, remove `column`'s filter.
    public mutating func setFilter(_ filter: Filter?, for column: ColumnID) {
        guard isVisible(column) else { return }
        guard let filter else {
            query.filters.removeAll { $0.column == column }
            return
        }
        if let index = query.filters.firstIndex(where: { $0.column == column }) {
            query.filters[index].filter = filter
        } else {
            query.filters.append(ColumnFilter(column: column, filter: filter))
            query.filters.sort { (order[$0.column] ?? .max) < (order[$1.column] ?? .max) }
        }
    }

    public mutating func clearFilters() {
        query.filters = []
    }

    /// Return the query to `applied`, such as after it failed to run.
    public mutating func revert(to applied: TableQuery) {
        query = applied
    }

    // MARK: Columns

    /// Reorder visible columns; `columns` must hold exactly the visible columns.
    public mutating func reorderColumns(_ columns: [ColumnID]) {
        guard columns.count == visibleColumns.count, Set(columns) == Set(visibleColumns) else { return }
        visibleColumns = columns
    }

    /// Show or hide `columns`. Shown columns go to the end in file order;
    /// hidden columns lose their filter and sort, and the column choices fall
    /// back to visible columns.
    public mutating func setColumnsVisible(_ columns: [ColumnID], visible: Bool) {
        let targets = Set(columns)
        if visible {
            let shown = Set(visibleColumns)
            visibleColumns += schema.filter { targets.contains($0) && !shown.contains($0) }
        } else {
            visibleColumns.removeAll(where: targets.contains)
        }
        let visible = Set(visibleColumns)
        query.filters.removeAll { !visible.contains($0.column) }
        query.sorts.removeAll { !visible.contains($0.column) }
        let fallback = visibleColumns.first
        if selectedColumn.map({ !isVisible($0) }) ?? true { selectedColumn = fallback }
        if inspectedColumn.map({ !isVisible($0) }) ?? true { inspectedColumn = fallback }
        if let chartX, !canPlot(chartX) { self.chartX = nil }
        if chartY.map({ !canPlot($0) }) ?? true { chartY = visibleColumns.first(where: canPlot) }
    }

    public mutating func showAllColumns() {
        setColumnsVisible(schema, visible: true)
    }
}
