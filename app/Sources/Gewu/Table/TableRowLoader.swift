import Foundation

/// A block of `TileKey.rowCount` display rows of one column, or of source row
/// indices when `column` is nil. Tiles start at multiples of `rowCount`.
struct TileKey: Hashable, Sendable {
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

    /// Tiles covering a viewport. Source rows come first; cell tiles are grouped
    /// by column so Rust can reuse each column's formatter.
    static func covering(rows: Range<Int>, columns: [ColumnId]) -> [TileKey] {
        guard !rows.isEmpty else { return [] }
        let blocks = (rows.lowerBound / rowCount)...((rows.upperBound - 1) / rowCount)
        return blocks.map { TileKey(block: $0, column: nil) }
            + columns.flatMap { column in blocks.map { TileKey(block: $0, column: column) } }
    }
}

/// Tiles read together. Each cell tile has its own single-column buffer, so
/// tiles are retained or evicted independently.
struct TableTiles: Sendable {
    var cells: [TileKey: CellText] = [:]
    var sourceRows: [Int: [UInt64]] = [:]

    func contains(_ key: TileKey) -> Bool {
        key.column == nil ? sourceRows[key.block] != nil : cells[key] != nil
    }

    var keys: [TileKey] { sourceRows.keys.map { TileKey(block: $0, column: nil) } + Array(cells.keys) }

    mutating func merge(_ other: TableTiles) {
        cells.merge(other.cells) { _, new in new }
        sourceRows.merge(other.sourceRows) { _, new in new }
    }

    /// Keep only tiles that intersect the viewport.
    mutating func retain(rows: Range<Int>, columns: [ColumnId]) {
        let columns = Set(columns)
        cells = cells.filter { key, _ in key.rows.overlaps(rows) && columns.contains(key.column!) }
        sourceRows = sourceRows.filter { block, _ in TileKey(block: block, column: nil).rows.overlaps(rows) }
    }
}

extension TableTiles {
    /// Read tiles in one FFI call. Tiles past the last row come back empty.
    init(reading view: GewuOrderedView, tiles: [TileKey]) throws {
        let count = UInt32(TileKey.rowCount)
        let blocks = tiles.filter { $0.column == nil }
        let cellKeys = tiles.filter { $0.column != nil }
        let batch = try view.readTiles(
            rows: blocks.map { RowSpan(start: UInt64($0.start), count: count) },
            cells: cellKeys.map { CellTile(column: $0.column!, start: UInt64($0.start), count: count) })
        precondition(batch.sourceRows.count == blocks.count && batch.cells.count == cellKeys.count)
        for (key, rows) in zip(blocks, batch.sourceRows) {
            precondition(rows.count <= TileKey.rowCount)
            sourceRows[key.block] = rows
        }
        for (key, text) in zip(cellKeys, batch.cells) {
            precondition(text.nCols == 1 && Int(text.nRows) <= TileKey.rowCount)
            cells[key] = text
        }
    }
}

/// Plans batched tile reads for the current viewport. Cells only read the
/// cache; scrolling replaces demand instead of appending work to a FIFO.
@MainActor
final class TableRowLoader {
    typealias Read = @Sendable (GewuOrderedView, [TileKey]) async throws -> TableTiles

    struct Viewport: Equatable {
        let rows: Range<Int>
        let columns: [ColumnId]
    }

    /// Tiles for a viewport of a View that is not displayed yet.
    struct Snapshot {
        let view: GewuOrderedView
        fileprivate let version: UInt64
        let viewport: Viewport
        let tiles: TableTiles
    }

    /// Each View gets a version. A tile can only be found under the version of
    /// the View it was read from, so a replaced View's tiles are never shown.
    private struct CacheKey: Hashable {
        let version: UInt64
        let tile: TileKey
    }

    private enum Tile {
        case cells(CellText)
        case sourceRows([UInt64])

        /// Buffers plus a fixed allowance for the entry itself.
        var cost: Int {
            switch self {
            case .cells(let text): return text.byteCount + 64
            case .sourceRows(let rows): return rows.count * MemoryLayout<UInt64>.stride + 64
            }
        }
    }

    var onLoad: ((GewuOrderedView, TableTiles) -> Void)?
    var onError: ((GewuOrderedView, Error) -> Void)?

    private let cache: LRUCache<CacheKey, Tile>
    private let read: Read
    private var view: GewuOrderedView?
    private var version: UInt64 = 0
    private var lastVersion: UInt64 = 0
    private var visibleRows = 0..<0
    private var visibleColumns: [ColumnId] = []
    // Completed demand, including uncached tiles, prevents cache pressure from
    // repeatedly loading the same viewport. Moving the viewport renews demand.
    private var delivered: Set<TileKey> = []
    private var failed: Set<TileKey> = []
    private var task: Task<Void, Never>?

    /// The defaults retain about 8 MiB. A tile above 1 MiB is shown when it is
    /// read but not retained, so a long value affects only its own tile.
    init(byteLimit: Int = 8 * 1024 * 1024, tileByteLimit: Int = 1024 * 1024, read: Read? = nil) {
        cache = LRUCache(costLimit: byteLimit, entryCostLimit: tileByteLimit)
        let reader = TableRowReader()
        self.read = read ?? { ordered, tiles in try await reader.read(ordered, tiles: tiles) }
    }

    var cachedTileCount: Int { cache.count }

    private func nextVersion() -> UInt64 {
        lastVersion &+= 1
        return lastVersion
    }

    /// Read only the target viewport, without invalidating the displayed cache.
    /// The snapshot holds its tiles, so values too large to cache are still committed.
    func snapshot(of ordered: GewuOrderedView, viewport: () -> Viewport) async throws -> Snapshot {
        let version = nextVersion()
        var tiles = TableTiles()
        var target = viewport()
        while true {
            try Task.checkCancellation()
            tiles.retain(rows: target.rows, columns: target.columns)
            let missing = TileKey.covering(rows: target.rows, columns: target.columns).filter { !tiles.contains($0) }
            if missing.isEmpty { return Snapshot(view: ordered, version: version, viewport: target, tiles: tiles) }
            tiles.merge(try await read(ordered, missing))
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
        record(snapshot.tiles, keys: snapshot.tiles.keys, version: snapshot.version)
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

    /// The cached tile containing `row`, and the row's index within it.
    func tile(for row: Int, column: ColumnId) -> (text: CellText, row: Int)? {
        guard row >= 0, case .cells(let text) = cache.value(for: key(row: row, column: column)) else { return nil }
        let local = row % TileKey.rowCount
        return local < Int(text.nRows) ? (text, local) : nil
    }

    func sourceRow(for row: Int) -> UInt64? {
        guard row >= 0, case .sourceRows(let rows) = cache.value(for: key(row: row, column: nil)) else { return nil }
        let local = row % TileKey.rowCount
        return local < rows.count ? rows[local] : nil
    }

    private func key(row: Int, column: ColumnId?) -> CacheKey {
        CacheKey(version: version, tile: TileKey(row: row, column: column))
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

    private func needed(_ key: TileKey) -> Bool {
        !failed.contains(key) && !delivered.contains(key)
            && !cache.contains(CacheKey(version: version, tile: key))
    }

    private func record(_ tiles: TableTiles, keys: [TileKey], version: UInt64) {
        delivered.formUnion(keys)
        func insert(_ tile: Tile, for key: TileKey) {
            cache.insert(tile, for: CacheKey(version: version, tile: key), cost: tile.cost)
        }
        for (block, rows) in tiles.sourceRows { insert(.sourceRows(rows), for: TileKey(block: block, column: nil)) }
        for (key, text) in tiles.cells { insert(.cells(text), for: key) }
    }

    private func loadNext() {
        guard task == nil, let view else { return }
        let missing = TileKey.covering(rows: visibleRows, columns: visibleColumns).filter(needed)
        guard !missing.isEmpty else { return }
        let read = read
        let version = version
        task = Task { [weak self] in
            do {
                let tiles = try await read(view, missing)
                guard !Task.isCancelled, let self, self.view === view, self.version == version else { return }
                self.task = nil
                self.record(tiles, keys: missing, version: version)
                self.onLoad?(view, tiles)
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.view === view, self.version == version else { return }
                self.task = nil
                // Suppress these tiles for the current demand, while allowing
                // other tiles to load. Moving away and back permits a retry.
                self.failed.formUnion(missing)
                self.onError?(view, error)
                self.loadNext()
            }
        }
    }
}

/// Serializes table reads off the main actor. Cancelled requests stop before
/// entering the synchronous FFI call; width measurement has its own serial worker.
actor TableRowReader {
    func read(_ ordered: GewuOrderedView, tiles: [TileKey]) throws -> TableTiles {
        try Task.checkCancellation()
        return try TableTiles(reading: ordered, tiles: tiles)
    }
}
