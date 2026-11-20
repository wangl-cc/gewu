import Foundation
import GewuKit

/// One column of the table over 64 aligned display rows: the unit the table
/// reads and caches. A table presentation choice, not a Core concept.
struct BlockKey: Hashable, Sendable {
    static let rowCount = 64

    let block: Int
    let column: ColumnID

    init(block: Int, column: ColumnID) {
        precondition(block >= 0)
        self.block = block
        self.column = column
    }

    init(row: Int, column: ColumnID) { self.init(block: row / Self.rowCount, column: column) }

    var start: Int { block * Self.rowCount }
    var rows: Range<Int> { start..<(start + Self.rowCount) }

    /// Keys covering a viewport, column by column.
    static func covering(rows: Range<Int>, columns: [ColumnID]) -> [BlockKey] {
        guard !rows.isEmpty else { return [] }
        let blocks = (rows.lowerBound / rowCount)...((rows.upperBound - 1) / rowCount)
        return columns.flatMap { column in blocks.map { BlockKey(block: $0, column: column) } }
    }
}

/// The rows and columns on screen.
struct Viewport: Equatable {
    var rows: Range<Int>
    var columns: [ColumnID]

    static let empty = Viewport(rows: 0..<0, columns: [])
}

/// Text blocks read for rows that are not displayed yet, so the table can
/// switch to them with the viewport already filled.
struct PreparedText {
    let rows: DisplayRows
    let viewport: Viewport
    fileprivate let blocks: [BlockKey: ColumnText]
}

/// Keeps the viewport's text available to cells and reuses recent blocks
/// when scrolling, for the rows the table displays.
///
/// Blocks on screen are held apart from the byte-bounded LRU, so cells can
/// rebind even when a block is too large to cache or was evicted. Scrolling
/// replaces pending demand; new rows drop all cached text.
@MainActor
final class TableTextCache {
    typealias Read = @Sendable (DisplayRows, BlockKey) async throws -> ColumnText

    /// Called with blocks that arrived for the displayed rows.
    var onLoad: (([BlockKey: ColumnText]) -> Void)?
    var onError: ((any Error) -> Void)?

    private let cache: LRUCache<BlockKey, ColumnText>
    private var visibleBlocks: [BlockKey: ColumnText] = [:]
    private let read: Read
    private(set) var rows: DisplayRows?
    private var viewport = Viewport.empty
    /// Blocks whose read failed for the current demand. Moving away and back retries them.
    private var failed: Set<BlockKey> = []
    private var task: Task<Void, Never>?

    init(
        byteLimit: Int = 8 * 1024 * 1024, blockByteLimit: Int = 1024 * 1024,
        read: @escaping Read = { rows, key in try await rows.text(of: key.column, rows: key.rows) }
    ) {
        cache = LRUCache(costLimit: byteLimit, entryCostLimit: blockByteLimit)
        self.read = read
    }

    var cachedBlockCount: Int { cache.count }

    /// Read the viewport of `rows` without disturbing the displayed text. The
    /// viewport is asked again after each read, since the table may scroll or
    /// resize meanwhile. A failed read leaves its block to load after the switch.
    func prepare(_ rows: DisplayRows, viewport: () -> Viewport) async throws -> PreparedText {
        var blocks: [BlockKey: ColumnText] = [:]
        var target = viewport()
        var failures: Set<BlockKey> = []
        while true {
            try Task.checkCancellation()
            let missing = BlockKey.covering(rows: target.rows, columns: target.columns).filter {
                blocks[$0] == nil && !failures.contains($0)
            }
            guard let key = missing.first else {
                return PreparedText(rows: rows, viewport: target, blocks: blocks)
            }
            do {
                blocks[key] = try await read(rows, key)
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                failures.insert(key)
            }
            target = viewport()
        }
    }

    /// Display prepared text, dropping everything cached for previous rows.
    func install(_ prepared: PreparedText) {
        reset(to: prepared.rows)
        viewport = prepared.viewport
        record(prepared.blocks)
    }

    /// Display `rows` with no text loaded yet.
    func reset(to rows: DisplayRows) {
        stop()
        self.rows = rows
    }

    func stop() {
        task?.cancel()
        task = nil
        rows = nil
        viewport = .empty
        failed.removeAll()
        cache.removeAll()
        visibleBlocks.removeAll()
    }

    /// The loaded block containing `row` of `column`.
    func text(row: Int, column: ColumnID) -> ColumnText? {
        guard row >= 0 else { return nil }
        let key = BlockKey(row: row, column: column)
        guard let text = visibleBlocks[key] ?? cache.value(for: key), text.rows.contains(row) else { return nil }
        return text
    }

    func updateViewport(_ requested: Viewport) {
        guard let rows else { return }
        let lower = min(rows.count, max(0, requested.rows.lowerBound))
        let upper = min(rows.count, max(lower, requested.rows.upperBound))
        var seen: Set<ColumnID> = []
        let next = Viewport(rows: lower..<upper, columns: requested.columns.filter { seen.insert($0).inserted })
        guard next != viewport else { return }
        let columns = Set(next.columns)
        failed = failed.filter { next.rows.overlaps($0.rows) && columns.contains($0.column) }
        visibleBlocks = visibleBlocks.filter { next.rows.overlaps($0.key.rows) && columns.contains($0.key.column) }
        viewport = next
        loadMissing()
    }

    private func record(_ blocks: [BlockKey: ColumnText]) {
        let columns = Set(viewport.columns)
        for (key, text) in blocks {
            if viewport.rows.overlaps(key.rows) && columns.contains(key.column) { visibleBlocks[key] = text }
            cache.insert(text, for: key, cost: text.byteCount + 64)
        }
    }

    private func loadMissing() {
        guard let rows, task == nil else { return }
        var missing: [BlockKey] = []
        for key in BlockKey.covering(rows: viewport.rows, columns: viewport.columns) where visibleBlocks[key] == nil {
            if let text = cache.value(for: key) {
                visibleBlocks[key] = text
            } else if !failed.contains(key) {
                missing.append(key)
            }
        }
        guard !missing.isEmpty else { return }
        let read = read
        task = Task {
            var loaded: [BlockKey: ColumnText] = [:]
            var failure: (any Error)?
            for key in missing {
                let result: Result<ColumnText, any Error>
                do { result = .success(try await read(rows, key)) } catch { result = .failure(error) }
                // New rows or a stop cancel this task; its blocks must not reach them.
                if Task.isCancelled { return }
                switch result {
                case .success(let text): loaded[key] = text
                case .failure(let error):
                    failed.insert(key)
                    failure = error
                }
            }
            task = nil
            record(loaded)
            if !loaded.isEmpty { onLoad?(loaded) }
            if let failure { onError?(failure) }
            loadMissing()
        }
    }
}
