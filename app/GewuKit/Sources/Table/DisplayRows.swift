import Foundation

/// The rows of a ``RowSet`` in display order: what a table shows.
///
/// Immutable. Text, search and export capture these exact rows, so they stay
/// consistent while newer queries replace the table's current rows.
public final class DisplayRows: Sendable, Hashable, Identifiable {
    public let rowSet: RowSet
    public let sorts: [ColumnSort]
    /// Number of display rows, equal to `rowSet.count`.
    public let count: Int
    let view: FfiOrderedView

    init(rowSet: RowSet, view: FfiOrderedView, sorts: [ColumnSort]) {
        self.rowSet = rowSet
        self.view = view
        self.sorts = sorts
        count = Int(clamping: view.rowCount())
    }

    public static func == (lhs: DisplayRows, rhs: DisplayRows) -> Bool { lhs === rhs }
    public func hash(into hasher: inout Hasher) { hasher.combine(ObjectIdentifier(self)) }

    public var source: TableSource { rowSet.source }

    /// The query these rows answer.
    public var query: TableQuery { TableQuery(filters: rowSet.filters, sorts: sorts) }

    public func sort(for column: ColumnID) -> ColumnSort? {
        sorts.first { $0.column == column }
    }

    private func clipped(_ rows: Range<Int>) -> (start: UInt64, count: UInt32) {
        let lower = min(max(rows.lowerBound, 0), count)
        let upper = min(max(rows.upperBound, lower), count)
        return (UInt64(lower), UInt32(clamping: upper - lower))
    }

    /// Display text of `column` for `rows`, clipped to the available rows.
    /// Formatting cost grows with the number and length of cells.
    public func text(of column: ColumnID, rows: Range<Int>) async throws -> ColumnText {
        let (start, count) = clipped(rows)
        let view = view
        return try await source.workers.read.perform {
            ColumnText(try view.formatColumn(column: column.ffi, start: start, count: count), start: Int(start))
        }
    }

    /// Zero-based source row numbers of `rows`, clipped to the available rows.
    /// A slice of the row mapping without formatting, fast enough for drawing.
    public func rowNumbers(_ rows: Range<Int>) -> RowNumbers {
        let (start, count) = clipped(rows)
        // Core fails only for a row index above UInt32.max, which no table can hold.
        return RowNumbers(try! view.sourceRows(start: start, count: count))
    }

    /// Prepare a search for `query` in `columns`, in that order. Compiling the
    /// query is immediate; navigation and highlights scan on demand.
    public func search(_ query: String, in columns: [ColumnID]) throws -> CellSearch {
        let session = try translatingErrors { try view.find(query: query, columns: columns.map(\.ffi)) }
        return CellSearch(session: session, rows: self, columns: columns)
    }

    /// Names of `columns` that only Parquet can export, because they are nested.
    public func nestedColumns(_ columns: [ColumnID]) throws -> [String] {
        try translatingErrors { try view.nestedExportColumns(columns: columns.map(\.ffi)) }
    }

    /// A single-use export of `columns`, in that order, over these rows.
    public func export(_ columns: [ColumnID], format: FileType) throws -> ExportJob {
        ExportJob(
            job: try translatingErrors { try view.export(columns: columns.map(\.ffi), format: format.ffi) },
            format: format)
    }
}
