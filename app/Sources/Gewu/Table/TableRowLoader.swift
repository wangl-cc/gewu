import Foundation

/// Owns row-block requests and their cache for one displayed View.
/// The controller decides what is visible and how completed blocks are rendered.
@MainActor
final class TableRowLoader {
    typealias Read = @Sendable (GewuView, RowCache.Key) async throws -> RowBlock

    var onLoad: ((GewuView, RowBlock, UInt32?) -> Void)?
    var onError: ((GewuView, Error) -> Void)?

    private let cache: RowCache
    private let read: Read
    private var view: GewuView?
    private var pending: [RowCache.Key] = []
    private var loading: RowCache.Key?
    private var task: Task<Void, Never>?
    private var failed = false

    init(cache: RowCache = RowCache(), read: Read? = nil) {
        self.cache = cache
        let reader = TableRowReader()
        self.read =
            read ?? { view, key in
                try await reader.read(
                    view, start: UInt64(key.start), count: UInt32(RowCache.blockSize),
                    columns: key.column.map { [$0] } ?? [])
            }
    }

    func reset(to view: GewuView) {
        stop()
        self.view = view
        cache.clear()
        failed = false
    }

    func stop() {
        task?.cancel()
        task = nil
        pending.removeAll()
        loading = nil
    }

    func cachedBlock(for key: RowCache.Key) -> RowBlock? {
        cache.block(for: key)
    }

    func block(for row: Int, column: UInt32?) -> RowBlock? {
        let key = RowCache.Key(start: row / RowCache.blockSize * RowCache.blockSize, column: column)
        if let block = cache.block(for: key) { return block }
        guard view != nil, !failed, loading != key, !pending.contains(key) else { return nil }
        // Completion asks the controller to revisit visible placeholders, so a full
        // queue does not permanently drop visible cells.
        if pending.count < 128 { pending.append(key) }
        loadNext()
        return nil
    }

    private func loadNext() {
        guard loading == nil, !pending.isEmpty, let view else { return }
        let key = pending.removeFirst()
        loading = key
        let read = read
        task = Task { [weak self] in
            do {
                let block = try await read(view, key)
                guard !Task.isCancelled, let self, self.view === view else { return }
                self.cache.insert(block, for: key)
                self.loading = nil
                self.task = nil
                // Deliver even oversized blocks that the cache cannot retain.
                self.onLoad?(view, block, key.column)
                self.loadNext()
            } catch {
                guard !Task.isCancelled, let self, self.view === view else { return }
                self.loading = nil
                self.task = nil
                self.pending.removeAll()
                self.failed = true
                self.onError?(view, error)
            }
        }
    }
}

/// Serializes table row reads off the main actor. Cancelled requests stop before
/// entering the synchronous FFI call; width measurement has its own serial worker.
actor TableRowReader {
    func read(_ view: GewuView, start: UInt64, count: UInt32, columns: [UInt32]) throws
        -> RowBlock
    {
        try Task.checkCancellation()
        return try view.rows(start: start, count: count, columns: columns)
    }
}
