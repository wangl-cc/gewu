import Foundation
internal import GewuBindings

/// The rows of a ``RowSet`` in display order: what a table shows.
///
/// Immutable. Text, Find and export capture these exact rows, so they stay
/// consistent while newer queries replace the table's current rows.
public final class DisplayRows: Sendable, Hashable, Identifiable {
    public let rowSet: RowSet
    public let sorts: [ColumnSort]
    /// Number of display rows: the rows of `rowSet`, in display order.
    public var count: Int { rowSet.count }
    /// The zero-based source row of every display row, read without calling into Rust.
    public let sourceRows: RowNumbers
    let view: GewuBindings.GewuOrderedView

    init(rowSet: RowSet, view: GewuBindings.GewuOrderedView, sorts: [ColumnSort]) {
        self.rowSet = rowSet
        self.view = view
        self.sorts = sorts
        sourceRows = RowNumbers(view)
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

    /// Source row numbers of `rows`, clipped to the available rows.
    public func rowNumbers(_ rows: Range<Int>) -> Slice<RowNumbers> {
        sourceRows[rows.clamped(to: 0..<count)]
    }

    /// Prepare Find for `query` in `columns`, in that order. Compiling the
    /// query is immediate; stepping and highlights scan on demand.
    public func find(_ query: String, in columns: [ColumnID]) throws -> CellFind {
        let find = try translatingErrors { try view.find(query: query, columns: columns.map(\.ffi)) }
        return CellFind(find: find, rows: self, columns: columns)
    }

    /// A single-use export of `columns`, in that order, over these rows.
    public func export(_ columns: [ColumnID], format: FileType) throws -> ExportJob {
        ExportJob(
            job: try translatingErrors { try view.export(columns: columns.map(\.ffi), format: format.ffi) },
            format: format)
    }
}
