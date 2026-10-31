import Foundation

/// A column-local LRU. Oversized values are delivered to visible cells but never
/// retained here; the table can display the full value without unbounded caching.
final class RowCache {
    struct Key: Hashable {
        let start: Int
        let column: UInt32?
    }

    static let blockSize = 64
    private let byteLimit: Int
    private let cellLimit: Int
    private var blocks: [Key: RowBlock] = [:]
    private var order: [Key] = []
    private(set) var byteCount = 0
    private(set) var cellCount = 0

    init(byteLimit: Int = 8 * 1024 * 1024, cellLimit: Int = 32_768) {
        self.byteLimit = byteLimit
        self.cellLimit = cellLimit
    }

    func clear() {
        blocks.removeAll()
        order.removeAll()
        byteCount = 0
        cellCount = 0
    }

    func block(for key: Key) -> RowBlock? {
        guard let block = blocks[key] else { return nil }
        order.removeAll { $0 == key }
        order.append(key)
        return block
    }

    @discardableResult
    func insert(_ block: RowBlock, for key: Key) -> Bool {
        let bytes = Self.bytes(block)
        let cells = Self.cells(block)
        guard bytes <= byteLimit, cells <= cellLimit else { return false }
        remove(key)
        while byteCount + bytes > byteLimit || cellCount + cells > cellLimit {
            guard let oldest = order.first else { break }
            remove(oldest)
        }
        blocks[key] = block
        order.append(key)
        byteCount += bytes
        cellCount += cells
        return true
    }

    private func remove(_ key: Key) {
        guard let old = blocks.removeValue(forKey: key) else { return }
        byteCount -= Self.bytes(old)
        cellCount -= Self.cells(old)
        order.removeAll { $0 == key }
    }

    private static func cells(_ block: RowBlock) -> Int {
        // Source-only requests must consume budget too.
        max(block.cells.count, block.sourceRows.count)
    }

    private static func bytes(_ block: RowBlock) -> Int {
        block.cells.reduce(0) { $0 + ($1?.utf8.count ?? 0) + MemoryLayout<String?>.stride }
            + block.sourceRows.count * MemoryLayout<UInt64>.stride
    }
}

/// Serializes table reads off the main actor, including width sampling and match
/// navigation. Cancelled requests stop before entering the synchronous FFI call.
actor TableRowReader {
    func read(_ document: GewuDocument, version: UInt64, start: UInt64, count: UInt32, columns: [UInt32]) throws
        -> RowBlock
    {
        try Task.checkCancellation()
        return try document.rows(version: version, start: start, count: count, columns: columns)
    }
}
