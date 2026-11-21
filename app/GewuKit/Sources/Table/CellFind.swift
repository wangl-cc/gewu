import Foundation
internal import GewuBindings

/// A cell in display coordinates.
public struct CellMatch: Hashable, Sendable {
    public let displayRow: Int
    public let column: ColumnID

    public init(displayRow: Int, column: ColumnID) {
        self.displayRow = displayRow
        self.column = column
    }

    init(_ match: GewuBindings.CellMatch) {
        displayRow = Int(match.displayRow)
        column = ColumnID(match.column)
    }

    var ffi: GewuBindings.CellMatch { GewuBindings.CellMatch(displayRow: UInt64(displayRow), column: column.ffi) }
}

/// Which cells of a block of display rows match, one bitmap per column.
public struct FindHighlight: Sendable {
    /// The display rows covered, clipped to the table.
    public let rows: Range<Int>
    /// One bit per row starting at `rows.lowerBound`, low bit first.
    public let matches: [ColumnID: [UInt8]]

    public func isMatch(row: Int, column: ColumnID) -> Bool {
        let local = row - rows.lowerBound
        guard rows.contains(row), let bits = matches[column], local / 8 < bits.count else { return false }
        return bits[local / 8] & (1 << (local % 8)) != 0
    }
}

/// Cell Find over one ``DisplayRows``: a compiled query scoped to ordered columns.
///
/// It reports no total: stepping moves through matches in display order,
/// scanning only as far as the match it returns, and highlights cover only
/// the blocks requested. A find keeps answering for its rows after the
/// table moves on to newer ones.
public final class CellFind: Sendable {
    public let rows: DisplayRows
    public let columns: [ColumnID]
    private let find: GewuBindings.GewuFind

    init(find: GewuBindings.GewuFind, rows: DisplayRows, columns: [ColumnID]) {
        self.find = find
        self.rows = rows
        self.columns = columns
    }

    /// The match `steps` matches after `cell` (before it when negative), in
    /// display order, wrapping around at either end. From `nil`, stepping by
    /// 1 finds the first match and by -1 the last. `nil` when nothing matches.
    public func step(_ steps: Int, from cell: CellMatch?) async throws -> CellMatch? {
        let find = find
        let steps = Int64(steps)
        return try await rows.source.workers.find.perform {
            try find.step(from: cell?.ffi, by: steps).map(CellMatch.init)
        }
    }

    /// Match flags for `columns`, which must belong to this find, over the
    /// display rows `range`, clipped to the table.
    public func highlight(_ columns: [ColumnID], in range: Range<Int>) async throws -> FindHighlight {
        let find = find
        let lower = min(max(range.lowerBound, 0), rows.count)
        let upper = min(max(range.upperBound, lower), rows.count)
        return try await rows.source.workers.find.perform { () throws -> FindHighlight in
            let result = try find.highlight(
                columns: columns.map(\.ffi), start: UInt64(lower), count: UInt32(clamping: upper - lower))
            var matches: [ColumnID: [UInt8]] = [:]
            for column in result.columns { matches[ColumnID(column.column)] = Array(column.matched) }
            return FindHighlight(rows: lower..<upper, matches: matches)
        }
    }
}
