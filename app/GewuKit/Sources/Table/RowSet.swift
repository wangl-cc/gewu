/// The rows that pass a set of filters, in file order.
///
/// A row set is immutable. Its analyses do not depend on row order, so a
/// sort change keeps the same row set and every analysis computed for it.
/// Results are computed on the session's analysis worker and shared among
/// all callers; see ``AnalysisCache``.
public final class RowSet: Sendable, Hashable, Identifiable {
    public let source: TableSource
    public let filters: [ColumnFilter]
    /// Number of rows that pass `filters`.
    public let count: UInt64
    let view: FfiFilteredView
    private let cache = AnalysisCache()

    init(source: TableSource, view: FfiFilteredView, filters: [ColumnFilter]) {
        self.source = source
        self.view = view
        self.filters = filters
        count = view.rowCount()
    }

    public static func == (lhs: RowSet, rhs: RowSet) -> Bool { lhs === rhs }
    public func hash(into hasher: inout Hasher) { hasher.combine(ObjectIdentifier(self)) }

    public func filter(for column: ColumnID) -> Filter? {
        filters.first { $0.column == column }?.filter
    }

    /// Synchronous; runs on the query worker.
    func sorted(by sorts: [ColumnSort]) throws -> DisplayRows {
        if sorts.isEmpty { return unsorted() }
        return DisplayRows(rowSet: self, view: try view.sort(sorts: sorts.map(\.ffi)), sorts: sorts)
    }

    func unsorted() -> DisplayRows {
        DisplayRows(rowSet: self, view: view.unsorted(), sorts: [])
    }

    // MARK: Analyses

    public func nullCount(_ column: ColumnID) async throws -> UInt64 {
        try await analyze(.nullCount(column)) { view in try view.countNulls(column: column.ffi) }
    }

    public func statistics(_ column: ColumnID) async throws -> NumericStatistics {
        try await analyze(.statistics(column)) { view in NumericStatistics(try view.statistics(column: column.ffi)) }
    }

    public func distribution(_ column: ColumnID, bins: UInt32, scale: AxisScale) async throws -> Histogram {
        try await analyze(.distribution(column, bins: bins, scale: scale)) { view in
            Histogram(try view.distribution(column: column.ffi, bins: bins, scale: scale.ffi))
        }
    }

    public func values(_ column: ColumnID) async throws -> ValueSummary {
        try await analyze(.values(column)) { view in ValueSummary(try view.values(column: column.ffi)) }
    }

    public func lengths(_ column: ColumnID, bins: UInt32) async throws -> LengthSummary {
        try await analyze(.lengths(column, bins: bins)) { view in
            LengthSummary(try view.lengths(column: column.ffi, bins: bins))
        }
    }

    /// A sample of up to `limit` pairs. `x == nil` plots against the source row number.
    public func scatter(
        x: ColumnID?, y: ColumnID, limit: UInt32, xScale: AxisScale, yScale: AxisScale
    ) async throws -> ScatterSample {
        try await analyze(.scatter(x: x, y: y, limit: limit, xScale: xScale, yScale: yScale)) { view in
            ScatterSample(
                try view.scatter(
                    xColumn: x?.ffi, yColumn: y.ffi, limit: limit, xScale: xScale.ffi, yScale: yScale.ffi))
        }
    }

    // MARK: Filter editors

    /// The values an editor for `column` can offer: those in rows passing
    /// every filter except `column`'s own, so its alternatives stay visible.
    public func filterOptions(for column: ColumnID) async throws -> FilterOptions {
        try await analyze(.filterOptions(column)) { view in
            FilterOptions(try view.facet(column: column.ffi).filterChoices(column: column.ffi))
        }
    }

    /// A histogram with exact bin endpoints for a range editor, over the same
    /// rows as ``filterOptions(for:)``. `nil` when the column has no plottable values.
    public func filterDistribution(_ column: ColumnID, bins: UInt32, scale: AxisScale) async throws -> RangeHistogram? {
        try await analyze(.filterDistribution(column, bins: bins, scale: scale)) { view in
            try view.facet(column: column.ffi).filterDistribution(column: column.ffi, bins: bins, scale: scale.ffi)
                .map(RangeHistogram.init)
        }
    }

    private func analyze<Value: Sendable>(
        _ key: AnalysisKey, _ compute: @escaping @Sendable (FfiFilteredView) throws -> Value
    ) async throws -> Value {
        let view = view
        let worker = source.workers.analysis
        return try await cache.value(for: key) {
            try await worker.perform { try compute(view) }
        }
    }
}
