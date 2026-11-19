import Foundation

/// Plans bounded reads for the current viewport. Cells only read the cache;
/// scrolling replaces demand instead of appending work to a FIFO.
@MainActor
final class TableRowLoader {
    typealias Read = @Sendable (GewuOrderedView, Int, [ColumnId]) async throws -> RowBlock

    struct Viewport: Equatable {
        let rows: Range<Int>
        let columns: [ColumnId]
    }

    struct Snapshot {
        struct Page {
            let block: RowBlock
            let columns: [ColumnId]
        }
        let view: GewuOrderedView
        let viewport: Viewport
        let pages: [Page]
    }

    /// The block is row-major, in the supplied column order. Even blocks
    /// too large for the cache are delivered to the visible cells once.
    var onLoad: ((GewuOrderedView, RowBlock, [ColumnId]) -> Void)?
    var onError: ((GewuOrderedView, Error) -> Void)?

    private struct Request {
        let start: Int
        let columns: [ColumnId]
    }

    private let cache: RowCache
    private let read: Read
    private var view: GewuOrderedView?
    private var visibleRows = 0..<0
    private var visibleColumns: [ColumnId] = []
    // Completed demand, including uncached blocks, prevents cache pressure from
    // repeatedly loading the same viewport. Moving the viewport renews demand.
    private var delivered: Set<RowCache.Key> = []
    private var failed: Set<RowCache.Key> = []
    private var task: Task<Void, Never>?

    init(cache: RowCache = RowCache(), read: Read? = nil) {
        self.cache = cache
        let reader = TableRowReader()
        self.read =
            read ?? { ordered, start, columns in
                let count = min(UInt64(RowCache.blockSize), ordered.rowCount() - UInt64(start))
                return try await reader.read(ordered, start: UInt64(start), count: UInt32(count), columns: columns)
            }
    }

    /// Read only the target viewport, without invalidating the displayed cache.
    /// The temporary pages also allow oversized values to be committed together.
    func snapshot(of ordered: GewuOrderedView, viewport: () -> Viewport) async throws -> Snapshot {
        var pages: [Snapshot.Page] = []
        var target = viewport()
        while true {
            try Task.checkCancellation()
            let columns = Set(target.columns)
            pages.removeAll { page in
                let start = Int(page.block.start)
                return !target.rows.overlaps(start..<(start + Int(page.block.nRows)))
                    || (!columns.isEmpty && columns.isDisjoint(with: page.columns))
            }
            var ready = Set<RowCache.Key>()
            for page in pages {
                let start = Int(page.block.start)
                ready.insert(.init(start: start, column: nil))
                for column in page.columns { ready.insert(.init(start: start, column: column)) }
            }
            let missing = Self.requests(in: target) { !ready.contains($0) }
            if missing.isEmpty { return Snapshot(view: ordered, viewport: target, pages: pages) }
            for request in missing {
                try Task.checkCancellation()
                let block = try await read(ordered, request.start, request.columns)
                pages.append(.init(block: block, columns: request.columns))
                let latest = viewport()
                if target != latest {
                    target = latest
                    break
                }
            }
        }
    }

    func install(_ snapshot: Snapshot) {
        reset(to: snapshot.view)
        visibleRows = snapshot.viewport.rows
        visibleColumns = snapshot.viewport.columns
        for page in snapshot.pages {
            record(page.block, columns: page.columns)
        }
    }

    func reset(to ordered: GewuOrderedView) {
        stop()
        view = ordered
        cache.clear()
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

    func cachedBlock(for key: RowCache.Key) -> RowBlock? {
        cache.block(for: key)
    }

    func block(for row: Int, column: ColumnId?) -> RowBlock? {
        guard row >= 0 else { return nil }
        let start = row / RowCache.blockSize * RowCache.blockSize
        if let block = cache.block(for: .init(start: start, column: column)) { return block }
        // Column blocks carry the same source IDs. Cache pressure may evict the
        // source-only entry before its columns; reuse their IDs in that case.
        if column == nil {
            for source in visibleColumns {
                if let block = cache.block(for: .init(start: start, column: source)) { return block }
            }
        }
        return nil
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
                    rows.overlaps(key.start..<(key.start + RowCache.blockSize))
                        && (key.column.map(sources.contains) ?? true)
                }
            }
            visibleRows = rows
            visibleColumns = columns
        }
        loadNext()
    }

    private func needed(_ key: RowCache.Key) -> Bool {
        !failed.contains(key) && !delivered.contains(key) && cache.block(for: key) == nil
    }

    private static func requests(in viewport: Viewport, needed: (RowCache.Key) -> Bool = { _ in true }) -> [Request] {
        guard !viewport.rows.isEmpty else { return [] }
        var requests: [Request] = []
        let first = viewport.rows.lowerBound / RowCache.blockSize * RowCache.blockSize
        for start in stride(from: first, to: viewport.rows.upperBound, by: RowCache.blockSize) {
            let columns = viewport.columns.filter { needed(.init(start: start, column: $0)) }
            if columns.isEmpty {
                if needed(.init(start: start, column: nil)) { requests.append(Request(start: start, columns: [])) }
            } else {
                for offset in stride(from: 0, to: columns.count, by: 16) {
                    requests.append(
                        Request(start: start, columns: Array(columns[offset..<min(offset + 16, columns.count)])))
                }
            }
        }
        return requests
    }

    private func record(_ block: RowBlock, columns: [ColumnId]) {
        let start = Int(block.start)
        delivered.insert(.init(start: start, column: nil))
        let source = RowBlock(start: block.start, nRows: block.nRows, nCols: 0, cells: [], sourceRows: block.sourceRows)
        cache.insert(source, for: .init(start: start, column: nil))
        for (index, column) in columns.enumerated() {
            delivered.insert(.init(start: start, column: column))
            let cells = (0..<Int(block.nRows)).map { block.cells[$0 * columns.count + index] }
            let slice = RowBlock(
                start: block.start, nRows: block.nRows, nCols: 1, cells: cells, sourceRows: block.sourceRows)
            cache.insert(slice, for: .init(start: start, column: column))
        }
    }

    private func loadNext() {
        guard task == nil, let view,
            let request = Self.requests(in: .init(rows: visibleRows, columns: visibleColumns), needed: needed).first
        else { return }
        let read = read
        task = Task { [weak self] in
            do {
                let block = try await read(view, request.start, request.columns)
                guard !Task.isCancelled, let self, self.view === view else { return }
                self.record(block, columns: request.columns)
                self.task = nil
                let end = request.start + Int(block.nRows)
                if request.start < self.visibleRows.upperBound && end > self.visibleRows.lowerBound {
                    self.onLoad?(view, block, request.columns)
                }
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.view === view else { return }
                self.task = nil
                // Suppress this request for the current demand, while allowing
                // unrelated pages to load. Moving away and back permits a retry.
                if request.columns.isEmpty { self.failed.insert(.init(start: request.start, column: nil)) }
                for column in request.columns {
                    self.failed.insert(.init(start: request.start, column: column))
                }
                self.onError?(view, error)
                self.loadNext()
            }
        }
    }
}

/// Serializes table row reads off the main actor. Cancelled requests stop before
/// entering the synchronous FFI call; width measurement has its own serial worker.
actor TableRowReader {
    func read(_ ordered: GewuOrderedView, start: UInt64, count: UInt32, columns: [ColumnId]) throws
        -> RowBlock
    {
        try Task.checkCancellation()
        return try ordered.rows(start: start, count: count, columns: columns)
    }
}
