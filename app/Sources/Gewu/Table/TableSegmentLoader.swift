import Foundation

/// One 64-row block of the table: a column's segment, or the block's source
/// row indices when `column` is nil. Blocks start at multiples of `rowCount`.
struct BlockKey: Hashable, Sendable {
    static let rowCount = 64

    let block: Int
    let column: ColumnId?

    init(block: Int, column: ColumnId?) {
        precondition(block >= 0)
        self.block = block
        self.column = column
    }

    init(row: Int, column: ColumnId?) { self.init(block: row / Self.rowCount, column: column) }

    var start: Int { block * Self.rowCount }
    var rows: Range<Int> { start..<(start + Self.rowCount) }

    /// Keys covering a viewport. Source rows come first, then each column's
    /// segments in order.
    static func covering(rows: Range<Int>, columns: [ColumnId]) -> [BlockKey] {
        guard !rows.isEmpty else { return [] }
        let blocks = (rows.lowerBound / rowCount)...((rows.upperBound - 1) / rowCount)
        return blocks.map { BlockKey(block: $0, column: nil) }
            + columns.flatMap { column in blocks.map { BlockKey(block: $0, column: column) } }
    }
}

/// Segments and source rows read together. Each segment has its own buffers,
/// so it is retained or evicted independently.
struct TableSegments: Sendable {
    var segments: [BlockKey: FormattedColumnSegment] = [:]
    var sourceRows: [Int: [UInt64]] = [:]

    func contains(_ key: BlockKey) -> Bool {
        key.column == nil ? sourceRows[key.block] != nil : segments[key] != nil
    }

    var keys: [BlockKey] { sourceRows.keys.map { BlockKey(block: $0, column: nil) } + Array(segments.keys) }

    mutating func merge(_ other: TableSegments) {
        segments.merge(other.segments) { _, new in new }
        sourceRows.merge(other.sourceRows) { _, new in new }
    }

    /// Keep only blocks that intersect the viewport.
    mutating func retain(rows: Range<Int>, columns: [ColumnId]) {
        let columns = Set(columns)
        segments = segments.filter { key, _ in key.rows.overlaps(rows) && columns.contains(key.column!) }
        sourceRows = sourceRows.filter { block, _ in BlockKey(block: block, column: nil).rows.overlaps(rows) }
    }
}

extension TableSegments {
    /// Read blocks in one FFI call. Blocks past the last row come back empty.
    init(reading view: GewuOrderedView, keys: [BlockKey]) throws {
        let count = UInt32(BlockKey.rowCount)
        let sourceKeys = keys.filter { $0.column == nil }
        let segmentKeys = keys.filter { $0.column != nil }
        let batch = try view.readSegments(
            rows: sourceKeys.map { RowSpan(start: UInt64($0.start), count: count) },
            segments: segmentKeys.map { ColumnSpan(column: $0.column!, start: UInt64($0.start), count: count) })
        precondition(batch.sourceRows.count == sourceKeys.count && batch.segments.count == segmentKeys.count)
        for (key, rows) in zip(sourceKeys, batch.sourceRows) {
            precondition(rows.count <= BlockKey.rowCount)
            sourceRows[key.block] = rows
        }
        for (key, segment) in zip(segmentKeys, batch.segments) {
            precondition(Int(segment.rowCount) <= BlockKey.rowCount)
            segments[key] = segment
        }
    }
}

/// Plans batched block reads for the current viewport. Cells only read the
/// cache; scrolling replaces demand instead of appending work to a FIFO.
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

    /// Each View gets a version. A block can only be found under the version of
    /// the View it was read from, so a replaced View's blocks are never shown.
    private struct CacheKey: Hashable {
        let version: UInt64
        let block: BlockKey
    }

    private enum Entry {
        case segment(FormattedColumnSegment)
        case sourceRows([UInt64])

        /// Buffers plus a fixed allowance for the entry itself.
        var cost: Int {
            switch self {
            case .segment(let segment): return segment.byteCount + 64
            case .sourceRows(let rows): return rows.count * MemoryLayout<UInt64>.stride + 64
            }
        }
    }

    var onLoad: ((GewuOrderedView, TableSegments) -> Void)?
    var onError: ((GewuOrderedView, Error) -> Void)?

    private let cache: LRUCache<CacheKey, Entry>
    private let read: Read
    private var view: GewuOrderedView?
    private var version: UInt64 = 0
    private var lastVersion: UInt64 = 0
    private var visibleRows = 0..<0
    private var visibleColumns: [ColumnId] = []
    // Completed demand, including uncached blocks, prevents cache pressure from
    // repeatedly loading the same viewport. Moving the viewport renews demand.
    private var delivered: Set<BlockKey> = []
    private var failed: Set<BlockKey> = []
    private var task: Task<Void, Never>?

    /// The defaults retain about 8 MiB. A segment above 1 MiB is shown when it
    /// is read but not retained, so a long value affects only its own segment.
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
        cache.removeAll { $0.version != snapshot.version }
        visibleRows = snapshot.viewport.rows
        visibleColumns = snapshot.viewport.columns
        record(snapshot.segments, keys: snapshot.segments.keys, version: snapshot.version)
    }

    func reset(to ordered: GewuOrderedView) {
        stop()
        view = ordered
        version = nextVersion()
        cache.removeAll()
    }

    func stop() {
        task?.cancel()
        task = nil
        view = nil
        visibleRows = 0..<0
        visibleColumns = []
        delivered.removeAll()
        failed.removeAll()
    }

    /// The cached segment containing `row`, and the row's index within it.
    func segment(for row: Int, column: ColumnId) -> (segment: FormattedColumnSegment, row: Int)? {
        guard row >= 0, case .segment(let segment) = cache.value(for: key(row: row, column: column)) else {
            return nil
        }
        let local = row % BlockKey.rowCount
        return local < Int(segment.rowCount) ? (segment, local) : nil
    }

    func sourceRow(for row: Int) -> UInt64? {
        guard row >= 0, case .sourceRows(let rows) = cache.value(for: key(row: row, column: nil)) else { return nil }
        let local = row % BlockKey.rowCount
        return local < rows.count ? rows[local] : nil
    }

    private func key(row: Int, column: ColumnId?) -> CacheKey {
        CacheKey(version: version, block: BlockKey(row: row, column: column))
    }

    func updateViewport(rows: Range<Int>, columns: [ColumnId]) {
        guard let view else { return }
        let count = Int(clamping: view.rowCount())
        let start = min(count, max(0, rows.lowerBound))
        let end = min(count, max(start, rows.upperBound))
        let rows = start..<end
        var seen: Set<ColumnId> = []
        let columns = columns.filter { seen.insert($0).inserted }
        if visibleRows != rows || visibleColumns != columns {
            // Installing a viewport includes bordering rows. Narrowing to the
            // actual visible region must not reread its uncached oversized values.
            if rows.lowerBound < visibleRows.lowerBound || rows.upperBound > visibleRows.upperBound
                || !Set(columns).isSubset(of: Set(visibleColumns))
            {
                delivered.removeAll()
            }
            if !failed.isEmpty {
                let sources = Set(columns)
                failed = failed.filter { key in
                    rows.overlaps(key.rows) && (key.column.map(sources.contains) ?? true)
                }
            }
            visibleRows = rows
            visibleColumns = columns
        }
        loadNext()
    }

    private func needed(_ key: BlockKey) -> Bool {
        !failed.contains(key) && !delivered.contains(key)
            && !cache.contains(CacheKey(version: version, block: key))
    }

    private func record(_ segments: TableSegments, keys: [BlockKey], version: UInt64) {
        delivered.formUnion(keys)
        func insert(_ entry: Entry, for key: BlockKey) {
            cache.insert(entry, for: CacheKey(version: version, block: key), cost: entry.cost)
        }
        for (block, rows) in segments.sourceRows { insert(.sourceRows(rows), for: BlockKey(block: block, column: nil)) }
        for (key, segment) in segments.segments { insert(.segment(segment), for: key) }
    }

    private func loadNext() {
        guard task == nil, let view else { return }
        let missing = BlockKey.covering(rows: visibleRows, columns: visibleColumns).filter(needed)
        guard !missing.isEmpty else { return }
        let read = read
        let version = version
        task = Task { [weak self] in
            do {
                let segments = try await read(view, missing)
                guard !Task.isCancelled, let self, self.view === view, self.version == version else { return }
                self.task = nil
                self.record(segments, keys: missing, version: version)
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
/// entering the synchronous FFI call; width measurement has its own serial worker.
actor TableSegmentReader {
    func read(_ ordered: GewuOrderedView, keys: [BlockKey]) throws -> TableSegments {
        try Task.checkCancellation()
        return try TableSegments(reading: ordered, keys: keys)
    }
}
