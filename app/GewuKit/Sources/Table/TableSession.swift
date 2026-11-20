import Foundation
internal import GewuBindings

/// One opened table: its data, and the workers that answer queries about it.
///
/// A session runs all its work off the main actor and keeps queries in
/// order. Each query result is an immutable ``DisplayRows``; the caller owns
/// which result is current and discards results it no longer needs.
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
            // Creating the unfiltered views is cheap but stays off the main actor with the read.
            let rows = RowSet(source: source, view: dataset.unfiltered(), filters: [])
            return TableSession(source: source, initialRows: rows.unsorted())
        }
    }

    /// Worksheet names of an XLSX workbook in workbook order, including hidden
    /// and empty sheets. Reads workbook metadata without decoding cells.
    public static func worksheetNames(_ url: URL) async throws -> [String] {
        let path = url.path
        return try await SerialWorker(label: "sheets").perform { try GewuBindings.xlsxSheetNames(path: path) }
    }

    /// The rows for `query`, reusing as much of `current` as possible: the
    /// same rows when nothing changed, and `current.rowSet` with its cached
    /// analyses when only the sort changed.
    public func apply(_ query: TableQuery, after current: DisplayRows) async throws -> DisplayRows {
        precondition(current.rowSet.source === source, "rows belong to another table")
        if query.filters == current.rowSet.filters {
            if query.sorts == current.sorts { return current }
            return try await sort(current.rowSet, by: query.sorts)
        }
        let source = source
        return try await source.workers.query.perform {
            let rows = try source.filter(query.filters)
            try Task.checkCancellation()
            return try rows.sorted(by: query.sorts)
        }
    }

    /// The rows of `rowSet` ordered by `sorts`.
    public func sort(_ rowSet: RowSet, by sorts: [ColumnSort]) async throws -> DisplayRows {
        try await source.workers.query.perform { try rowSet.sorted(by: sorts) }
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

    /// Synchronous; runs on the query worker.
    fileprivate func filter(_ filters: [ColumnFilter]) throws -> RowSet {
        RowSet(source: self, view: try dataset.filter(filters: filters.map(\.ffi)), filters: filters)
    }
}
