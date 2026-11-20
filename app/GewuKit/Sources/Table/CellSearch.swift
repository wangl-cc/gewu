import Foundation

/// A cell in display coordinates.
public struct CellMatch: Hashable, Sendable {
    public let displayRow: Int
    public let column: ColumnID

    public init(displayRow: Int, column: ColumnID) {
        self.displayRow = displayRow
        self.column = column
    }

    init(_ match: FfiCellMatch) {
        displayRow = Int(match.displayRow)
        column = ColumnID(match.column)
    }

    var ffi: FfiCellMatch { FfiCellMatch(displayRow: UInt64(displayRow), column: column.ffi) }
}

/// Which cells of a block of display rows match, one bitmap per column.
public struct SearchHighlights: Sendable {
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

/// A compiled search over one ``DisplayRows``, scoped to ordered columns.
///
/// It reports no total: navigation pages through matches in display order,
/// and highlights cover only the blocks requested. A search keeps answering
/// for its rows after the table moves on to newer ones.
public final class CellSearch: Sendable {
    /// Matches returned by one navigation call at most.
    public static let maximumPageSize = Int(findLimits().maxPageSize)
    /// Display rows covered by one highlight call at most.
    public static let maximumHighlightRows = Int(findLimits().maxHighlightRows)

    public let rows: DisplayRows
    public let columns: [ColumnID]
    private let session: FfiFindSession

    init(session: FfiFindSession, rows: DisplayRows, columns: [ColumnID]) {
        self.session = session
        self.rows = rows
        self.columns = columns
    }

    /// Up to `limit` matches strictly after `cell` (from the start when `nil`),
    /// in display order. Empty at the end.
    public func matches(after cell: CellMatch?, limit: Int) async throws -> [CellMatch] {
        let session = session
        let limit = UInt32(clamping: limit)
        return try await rows.source.workers.find.perform {
            try session.next(after: cell?.ffi, limit: limit).map(CellMatch.init)
        }
    }

    /// Up to `limit` matches strictly before `cell` (from the end when `nil`),
    /// in display order. Empty at the start.
    public func matches(before cell: CellMatch?, limit: Int) async throws -> [CellMatch] {
        let session = session
        let limit = UInt32(clamping: limit)
        return try await rows.source.workers.find.perform {
            try session.previous(before: cell?.ffi, limit: limit).map(CellMatch.init)
        }
    }

    /// Match flags for `columns`, which must belong to this search, over at
    /// most ``maximumHighlightRows`` display rows.
    public func highlights(_ columns: [ColumnID], rows: Range<Int>) async throws -> SearchHighlights {
        let session = session
        let lower = min(max(rows.lowerBound, 0), self.rows.count)
        let upper = min(max(rows.upperBound, lower), self.rows.count)
        return try await self.rows.source.workers.find.perform { () throws -> SearchHighlights in
            let result = try session.highlight(
                columns: columns.map(\.ffi), start: UInt64(lower), count: UInt32(upper - lower))
            var matches: [ColumnID: [UInt8]] = [:]
            for column in result.columns { matches[ColumnID(column.column)] = Array(column.matched) }
            return SearchHighlights(rows: lower..<upper, matches: matches)
        }
    }
}
