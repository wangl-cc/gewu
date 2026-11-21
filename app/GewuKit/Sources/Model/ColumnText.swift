import Foundation
internal import GewuBindings
internal import gewuFFI

/// Display text for one column over consecutive display rows. A plain value:
/// Rust formats a block and moves it across once, packed as row offsets, a
/// validity bitmap and the UTF-8 text. Decode only the cells being shown.
public struct ColumnText: Sendable {
    /// Rows in this block, starting at ``DisplayRows`` row `start`.
    public let rowCount: Int
    public let start: Int
    /// `rowCount + 1` native-endian `UInt32` offsets, validity bits, then text.
    private let packed: Data
    private let textStart: Int

    init(_ text: GewuBindings.ColumnText, start: Int) {
        rowCount = Int(text.rowCount)
        self.start = start
        packed = text.bytes
        textStart = (rowCount + 1) * MemoryLayout<UInt32>.size + (rowCount + 7) / 8
        precondition(packed.count >= textStart, "column text is shorter than its header")
    }

    /// The display rows this block covers.
    public var rows: Range<Int> { start..<(start + rowCount) }

    /// Bytes retained by this block.
    public var byteCount: Int { packed.count }

    /// Text of display row `row`; `nil` for a missing value or a row outside this block.
    public func cell(_ row: Int) -> String? {
        withUTF8(row) { String(decoding: $0, as: UTF8.self) }
    }

    /// Run `body` on the UTF-8 bytes of display row `row` without copying them.
    /// Returns `nil` for a missing value or a row outside this block.
    public func withUTF8<Result>(_ row: Int, _ body: (UnsafeRawBufferPointer) throws -> Result) rethrows
        -> Result?
    {
        let local = row - start
        guard local >= 0, local < rowCount else { return nil }
        let validityStart = (rowCount + 1) * MemoryLayout<UInt32>.size
        let textStart = textStart
        return try packed.withUnsafeBytes { bytes in
            guard bytes[validityStart + local / 8] & (1 << (local % 8)) != 0 else { return nil }
            let lower = Int(bytes.loadUnaligned(fromByteOffset: local * 4, as: UInt32.self))
            let upper = Int(bytes.loadUnaligned(fromByteOffset: (local + 1) * 4, as: UInt32.self))
            return try body(UnsafeRawBufferPointer(rebasing: bytes[(textStart + lower)..<(textStart + upper)]))
        }
    }
}

/// Zero-based source row numbers of every display row, read in place from
/// the row mapping Rust keeps for the rows. Reading never calls into Rust.
public struct RowNumbers: @unchecked Sendable, RandomAccessCollection {
    // SAFETY of `@unchecked Sendable`: `rows` points into an immutable
    // mapping that `owner` keeps alive, so concurrent reads are safe.
    /// Keeps the mapping alive; `nil` when every row is in file order.
    private let owner: GewuBindings.GewuOrderedView?
    private let rows: UnsafeBufferPointer<UInt32>?
    public let count: Int

    init(_ view: GewuBindings.GewuOrderedView) {
        let lent = gewu_ordered_view_source_rows(view.uniffiCloneHandle())
        count = lent.count
        if let start = lent.rows {
            owner = view
            rows = UnsafeBufferPointer(start: start, count: lent.count)
        } else {
            // File order needs no mapping, so there is nothing to keep alive.
            owner = nil
            rows = nil
        }
    }

    public var startIndex: Int { 0 }
    public var endIndex: Int { count }

    public subscript(position: Int) -> UInt64 {
        precondition(position >= 0 && position < count, "row number index out of range")
        guard let rows else { return UInt64(position) }
        return withExtendedLifetime(owner) { UInt64(rows[position]) }
    }
}
