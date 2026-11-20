/// An immutable buffer owned by a Rust object. The address stays valid while
/// `owner` is alive; Rust never mutates or reallocates it.
struct ForeignBuffer: Sendable {
    private let owner: any Sendable
    private let address: UInt
    let byteCount: Int

    init(_ view: FfiBufferView, owner: any Sendable) {
        self.owner = owner
        address = UInt(view.address)
        byteCount = Int(view.length)
    }

    /// The bytes, valid only within `body`, which must not let them escape.
    func withBytes<Result>(_ body: (UnsafeRawBufferPointer) throws -> Result) rethrows -> Result {
        try withExtendedLifetime(owner) {
            // SAFETY: Rust keeps the buffer at this address, unchanged, for the
            // owner's lifetime, which `withExtendedLifetime` covers. Empty
            // buffers are never dereferenced.
            try body(UnsafeRawBufferPointer(start: UnsafeRawPointer(bitPattern: address), count: byteCount))
        }
    }
}

/// Display text for one column over consecutive display rows, read in place
/// from Rust memory. Decode only the cells being shown.
public struct ColumnText: Sendable {
    /// Rows in this block, starting at ``DisplayRows`` row `start`.
    public let rowCount: Int
    public let start: Int
    private let text: ForeignBuffer
    private let offsets: ForeignBuffer
    private let validity: ForeignBuffer

    init(_ handle: FfiColumnText, start: Int) {
        rowCount = Int(handle.rowCount())
        self.start = start
        text = ForeignBuffer(handle.text(), owner: handle)
        offsets = ForeignBuffer(handle.offsets(), owner: handle)
        validity = ForeignBuffer(handle.validity(), owner: handle)
    }

    /// The display rows this block covers.
    public var rows: Range<Int> { start..<(start + rowCount) }

    /// Bytes retained by this block's buffers.
    public var byteCount: Int { text.byteCount + offsets.byteCount + validity.byteCount }

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
        let valid = validity.withBytes { $0[local / 8] & (1 << (local % 8)) != 0 }
        guard valid else { return nil }
        let (lower, upper) = offsets.withBytes { bytes in
            let words = bytes.assumingMemoryBound(to: UInt32.self)
            return (Int(words[local]), Int(words[local + 1]))
        }
        return try text.withBytes { try body(UnsafeRawBufferPointer(rebasing: $0[lower..<upper])) }
    }
}

/// Zero-based source row numbers for consecutive display rows.
public struct RowNumbers: Sendable, RandomAccessCollection {
    private let rows: ForeignBuffer
    public let count: Int

    init(_ handle: FfiRowIndices) {
        rows = ForeignBuffer(handle.rows(), owner: handle)
        count = Int(handle.count())
    }

    public var startIndex: Int { 0 }
    public var endIndex: Int { count }

    public subscript(position: Int) -> UInt64 {
        precondition(position >= 0 && position < count, "row number index out of range")
        return rows.withBytes { UInt64($0.assumingMemoryBound(to: UInt32.self)[position]) }
    }
}
