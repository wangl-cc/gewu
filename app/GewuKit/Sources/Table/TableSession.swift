import Foundation
internal import GewuBindings

/// One opened table: its data, and the workers that answer queries about it.
///
/// A session runs its queries, analyses, text reads and Find each on a
/// worker of its own, off the main actor. Each query result is an immutable
/// ``DisplayRows``; the caller owns which result is current and discards
/// results it no longer needs.
public final class TableSession: Sendable {
    public let source: TableSource
    /// All rows in file order, the result of an empty ``TableQuery``.
    public let initialRows: DisplayRows

    private init(source: TableSource, initialRows: DisplayRows) {
        self.source = source
        self.initialRows = initialRows
    }

    /// Read and decode a file. Cancelling before the read starts skips it;
    /// a read that has started runs to completion.
    public static func open(_ url: URL, options: OpenOptions = OpenOptions()) async throws -> TableSession {
        let workers = Workers()
        let path = url.path
        return try await workers.query.perform {
            let dataset = try GewuBindings.GewuDataset.open(
                path: path, options: GewuBindings.OpenOptions(sheet: options.sheet, hasHeader: options.hasHeader))
            let source = TableSource(dataset: dataset, workers: workers)
            // Creating the initial views is cheap but stays off the main actor with the read.
            let view = dataset.rows()
            let rows = RowSet(source: source, view: view.filtered(), filters: [])
            return TableSession(source: source, initialRows: DisplayRows(rowSet: rows, view: view, sorts: []))
        }
    }

    /// Worksheet names of an XLSX workbook in workbook order, including hidden
    /// and empty sheets. Reads workbook metadata without decoding cells.
    public static func worksheetNames(_ url: URL) async throws -> [String] {
        let path = url.path
        return try await SerialWorker.perform(alone: "sheets") { try GewuBindings.xlsxSheetNames(path: path) }
    }

    /// The rows for `query`, reusing as much of `current` as the engine can:
    /// `current` itself when nothing changed, and `current.rowSet` with its
    /// analyses when only the sort changed. Filters always apply to the whole
    /// table, not to `current`. The engine alone decides what is reused;
    /// GewuKit keeps the matching Swift results.
    public func apply(_ query: TableQuery, after current: DisplayRows) async throws -> DisplayRows {
        precondition(current.rowSet.source === source, "rows belong to another table")
        let source = source
        return try await source.workers.query.perform {
            let view = try current.view.apply(filters: query.filters.map(\.ffi), sorts: query.sorts.map(\.ffi))
            if view.isSameView(other: current.view) { return current }
            let filtered = view.filtered()
            let rowSet =
                filtered.isSameView(other: current.rowSet.view)
                ? current.rowSet : RowSet(source: source, view: filtered, filters: query.filters)
            return DisplayRows(rowSet: rowSet, view: view, sorts: query.sorts)
        }
    }
}

/// What a file contains. Shared by every query result of its session.
public final class TableSource: Sendable {
    public let info: TableInfo
    let dataset: GewuBindings.GewuDataset
    let workers: Workers
    private let columnsByID: [ColumnID: ColumnInfo]

    init(dataset: GewuBindings.GewuDataset, workers: Workers) {
        self.dataset = dataset
        self.workers = workers
        info = TableInfo(dataset.info())
        columnsByID = Dictionary(uniqueKeysWithValues: info.columns.map { ($0.id, $0) })
    }

    public var columns: [ColumnInfo] { info.columns }

    public func column(_ id: ColumnID) -> ColumnInfo? { columnsByID[id] }

    /// Nested fields of a column, read from the schema without scanning.
    public func structure(of column: ColumnID) throws -> [AnalysisField] {
        try translatingErrors { try dataset.columnStructure(column: column.ffi).map(AnalysisField.init) }
    }
}
