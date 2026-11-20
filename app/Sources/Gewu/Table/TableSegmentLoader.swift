import Foundation

/// One column of the table over 64 aligned display rows: the unit the table
/// reads and caches. Blocks start at multiples of `rowCount`.
struct BlockKey: Hashable, Sendable {
    static let rowCount = 64

    let block: Int
    let column: ColumnId

    init(block: Int, column: ColumnId) {
        precondition(block >= 0)
        self.block = block
        self.column = column
    }

    init(row: Int, column: ColumnId) { self.init(block: row / Self.rowCount, column: column) }

    var start: Int { block * Self.rowCount }
    var rows: Range<Int> { start..<(start + Self.rowCount) }

    /// Keys covering a viewport, column by column.
    static func covering(rows: Range<Int>, columns: [ColumnId]) -> [BlockKey] {
        guard !rows.isEmpty else { return [] }
        let blocks = (rows.lowerBound / rowCount)...((rows.upperBound - 1) / rowCount)
        return columns.flatMap { column in blocks.map { BlockKey(block: $0, column: column) } }
    }
}

/// Column segments ready to display, each with its own buffers.
struct TableSegments: Sendable {
    var segments: [BlockKey: FormattedColumnSegment] = [:]

    func contains(_ key: BlockKey) -> Bool { segments[key] != nil }

    var keys: [BlockKey] { Array(segments.keys) }

    mutating func merge(_ other: TableSegments) {
        segments.merge(other.segments) { _, new in new }
    }

    /// Keep only blocks that intersect the viewport.
    mutating func retain(rows: Range<Int>, columns: [ColumnId]) {
        let columns = Set(columns)
        segments = segments.filter { key, _ in key.rows.overlaps(rows) && columns.contains(key.column) }
    }
}

extension TableSegments {
    /// Format the requested column segments.
    init(reading view: GewuOrderedView, keys: [BlockKey]) throws {
        let count = UInt32(BlockKey.rowCount)
        for key in keys {
            try Task.checkCancellation()
            segments[key] = try view.formatColumn(column: key.column, start: UInt64(key.start), count: count)
        }
    }
}

/// Keeps the current viewport's blocks available to cells and reuses recent
/// column segments when scrolling.
@MainActor
final class TableSegmentLoader {
    typealias Read = @Sendable (GewuOrderedView, [BlockKey]) async throws -> TableSegments

    struct Viewport: Equatable {
        let rows: Range<Int>
        let columns: [ColumnId]
    }

    /// Blocks for a viewport of a View that is not displayed yet.
    struct Snapshot {
        let view: GewuOrderedView
        fileprivate let version: UInt64
        let viewport: Viewport
        let segments: TableSegments
    }

    var onLoad: ((GewuOrderedView, TableSegments) -> Void)?
    var onError: ((GewuOrderedView, Error) -> Void)?

    private let cache: LRUCache<BlockKey, FormattedColumnSegment>
    private var viewportSegments = TableSegments()
    private let read: Read
    private var view: GewuOrderedView?
    private var version: UInt64 = 0
    private var lastVersion: UInt64 = 0
    private var visibleRows = 0..<0
    private var visibleColumns: [ColumnId] = []
    private var failed: Set<BlockKey> = []
    private var task: Task<Void, Never>?

    /// The viewport keeps each visible segment available independently of the LRU.
    init(byteLimit: Int = 8 * 1024 * 1024, segmentByteLimit: Int = 1024 * 1024, read: Read? = nil) {
        cache = LRUCache(costLimit: byteLimit, entryCostLimit: segmentByteLimit)
        let reader = TableSegmentReader()
        self.read = read ?? { ordered, keys in try await reader.read(ordered, keys: keys) }
    }

    var cachedEntryCount: Int { cache.count }

    private func nextVersion() -> UInt64 {
        lastVersion &+= 1
        return lastVersion
    }

    /// Read only the target viewport, without invalidating the displayed cache.
    /// The snapshot holds its segments, so values too large to cache are still committed.
    func snapshot(of ordered: GewuOrderedView, viewport: () -> Viewport) async throws -> Snapshot {
        let version = nextVersion()
        var segments = TableSegments()
        var target = viewport()
        while true {
            try Task.checkCancellation()
            segments.retain(rows: target.rows, columns: target.columns)
            let missing = BlockKey.covering(rows: target.rows, columns: target.columns).filter {
                !segments.contains($0)
            }
            if missing.isEmpty {
                return Snapshot(view: ordered, version: version, viewport: target, segments: segments)
            }
            segments.merge(try await read(ordered, missing))
            target = viewport()
        }
    }

    func install(_ snapshot: Snapshot) {
        stop()
        view = snapshot.view
        version = snapshot.version
        visibleRows = snapshot.viewport.rows
        visibleColumns = snapshot.viewport.columns
        record(snapshot.segments)
    }

    func reset(to ordered: GewuOrderedView) {
        stop()
        view = ordered
        version = nextVersion()
    }

    func stop() {
        task?.cancel()
        task = nil
        view = nil
        visibleRows = 0..<0
        visibleColumns = []
        failed.removeAll()
        cache.removeAll()
        viewportSegments = TableSegments()
    }

    /// The available segment containing `row`, and the row's index within it.
    func segment(for row: Int, column: ColumnId) -> (segment: FormattedColumnSegment, row: Int)? {
        guard row >= 0 else { return nil }
        let key = BlockKey(row: row, column: column)
        let local = row - key.start
        guard let segment = viewportSegments.segments[key] ?? cache.value(for: key),
            local < Int(segment.rowCount)
        else {
            return nil
        }
        return (segment, local)
    }

    func updateViewport(rows: Range<Int>, columns: [ColumnId]) {
        guard let view else { return }
        let count = Int(clamping: view.rowCount())
        let start = min(count, max(0, rows.lowerBound))
        let end = min(count, max(start, rows.upperBound))
        let rows = start..<end
        var seen: Set<ColumnId> = []
        let columns = columns.filter { seen.insert($0).inserted }
        guard visibleRows != rows || visibleColumns != columns else { return }
        if !failed.isEmpty {
            let sources = Set(columns)
            failed = failed.filter { key in rows.overlaps(key.rows) && sources.contains(key.column) }
        }
        visibleRows = rows
        visibleColumns = columns
        viewportSegments.retain(rows: rows, columns: columns)
        loadNext()
    }

    private func record(_ segments: TableSegments) {
        var visible = segments
        visible.retain(rows: visibleRows, columns: visibleColumns)
        viewportSegments.merge(visible)
        for (key, segment) in segments.segments {
            cache.insert(segment, for: key, cost: segment.byteCount + 64)
        }
    }

    private func loadNext() {
        guard let view else { return }
        var missing: [BlockKey] = []
        for key in BlockKey.covering(rows: visibleRows, columns: visibleColumns) where !viewportSegments.contains(key) {
            if let segment = cache.value(for: key) {
                viewportSegments.segments[key] = segment
            } else if !failed.contains(key) {
                missing.append(key)
            }
        }
        guard task == nil, !missing.isEmpty else { return }
        let read = read
        let version = version
        task = Task { [weak self] in
            do {
                let segments = try await read(view, missing)
                guard !Task.isCancelled, let self, self.view === view, self.version == version else { return }
                self.task = nil
                self.record(segments)
                self.onLoad?(view, segments)
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.view === view, self.version == version else { return }
                self.task = nil
                // Suppress these blocks for the current demand, while allowing
                // other blocks to load. Moving away and back permits a retry.
                self.failed.formUnion(missing)
                self.onError?(view, error)
                self.loadNext()
            }
        }
    }
}

/// Serializes table reads off the main actor. Cancelled requests stop before
/// entering each synchronous FFI call. Width measurement has its own serial worker.
actor TableSegmentReader {
    func read(_ ordered: GewuOrderedView, keys: [BlockKey]) throws -> TableSegments {
        return try TableSegments(reading: ordered, keys: keys)
    }
}
