import Foundation

/// One window's query state: the applied View, the spec it came from, and the
/// results computed for that View.
///
/// A bounded set of results is cached per applied View and shared by every panel. Replacing the
/// View starts a new epoch: work still running for the old View is cancelled and
/// its results are released. Reordering columns keeps the epoch, because no cached
/// result depends on column order. Views stay immutable and can outlive their
/// epoch, so export and copy capture a View directly instead of going through here.
@MainActor
final class Viewer {
    let dataset: GewuDataset

    struct Applied {
        let view: GewuView
        let spec: QuerySpec
    }

    enum Outcome {
        case applied
        /// A newer request replaced this one, or its task was cancelled.
        case superseded
        /// The latest request was cancelled through Core; the applied View is unchanged.
        case cancelled
        /// The latest request failed; the applied View is unchanged.
        case failed(Error)
    }

    private struct DistributionKey: Hashable {
        let column: UInt32
        let bins: UInt32
        let scale: AxisScale
    }

    private struct LengthKey: Hashable {
        let column: UInt32
        let bins: UInt32
    }

    private struct ScatterKey: Hashable {
        let x: UInt32?
        let y: UInt32
        let limit: UInt32
        let xScale: AxisScale
        let yScale: AxisScale
    }

    @MainActor
    private final class ResultCache<Key: Hashable, Value: Sendable> {
        var tasks: [Key: Task<Value, Error>] = [:]
    }

    /// Parameters select a result within a slot; changing them replaces that slot.
    private enum Slot: Hashable {
        case facet(UInt32)
        case choices(UInt32)
        case filterDistribution(UInt32)
        case nulls(UInt32)
        case statistics(UInt32)
        case distribution(UInt32)
        case values(UInt32)
        case lengths(UInt32)
        case scatter
    }

    /// Work and results that belong to one applied View.
    @MainActor
    private final class Epoch {
        let facets = ResultCache<UInt32, GewuView>()
        let filterChoices = ResultCache<UInt32, FilterChoices>()
        let filterDistributions = ResultCache<DistributionKey, RangeHistogram?>()
        let nullCounts = ResultCache<UInt32, UInt64>()
        let statistics = ResultCache<UInt32, NumericStatistics>()
        let distributions = ResultCache<DistributionKey, Histogram>()
        let values = ResultCache<UInt32, ValueSummary>()
        let lengths = ResultCache<LengthKey, LengthSummary>()
        let scatter = ResultCache<ScatterKey, ScatterResult>()

        // Include pending work in the budget, so rapid navigation cannot queue unlimited scans.
        private var recent: [Slot] = []
        private var removals: [Slot: (Bool) -> Void] = [:]

        func touch(_ slot: Slot) {
            recent.removeAll { $0 == slot }
            recent.append(slot)
        }

        func insert(_ slot: Slot, remove: @escaping (Bool) -> Void) {
            removals[slot] = remove
            touch(slot)
            if recent.count > 32 { discard(recent[0]) }
        }

        func discard(_ slot: Slot, cancel: Bool = true) {
            removals.removeValue(forKey: slot)?(cancel)
            recent.removeAll { $0 == slot }
        }

        func cancel() {
            for remove in removals.values { remove(true) }
            removals.removeAll()
            recent.removeAll()
        }
    }

    private(set) var applied: Applied {
        willSet { willChange?() }
    }
    /// Called before the applied View or spec changes, so an observing model can publish it.
    var willChange: (() -> Void)?
    private var epoch = Epoch()
    private var generation: UInt64 = 0
    private var pending: Cancellation?
    // Bound concurrent queries; the generation decides which completed query applies.
    private let worker = DispatchQueue(label: "gewu.viewer.queries", qos: .userInitiated)
    private let analysisWorker: OperationQueue

    init(dataset: GewuDataset, view: GewuView, spec: QuerySpec, analysisWorker: OperationQueue? = nil) {
        self.dataset = dataset
        applied = Applied(view: view, spec: spec)
        self.analysisWorker = analysisWorker ?? OperationQueue()
        self.analysisWorker.name = "gewu.viewer.analysis"
        self.analysisWorker.qualityOfService = .userInitiated
        self.analysisWorker.maxConcurrentOperationCount = 1
    }

    /// Compute a complete spec from the full dataset and apply it. The most
    /// recently started request wins; a failed or superseded request keeps the
    /// applied View.
    func apply(_ spec: QuerySpec, debounce: Bool = false) async -> Outcome {
        generation &+= 1
        let mine = generation
        pending?.cancel()
        let signal = Cancellation()
        pending = signal
        let dataset = dataset
        let worker = worker
        do {
            if debounce { try await Task.sleep(for: .milliseconds(150)) }
            try Task.checkCancellation()
            guard mine == generation else { return .superseded }
            let view: GewuView = try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        do {
                            continuation.resume(returning: try dataset.query(spec: spec, cancellation: signal))
                        } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
            } onCancel: {
                signal.cancel()
            }
            guard !Task.isCancelled, mine == generation else { return .superseded }
            pending = nil
            replace(Applied(view: view, spec: spec))
            return .applied
        } catch {
            guard !Task.isCancelled, mine == generation else { return .superseded }
            pending = nil
            return error.isCancellation ? .cancelled : .failed(error)
        }
    }

    /// Show the same rows in another column order. Cached results stay valid.
    func reorder(_ columns: [UInt32], spec: QuerySpec) throws {
        let view = try applied.view.reorderColumns(order: columns)
        applied = Applied(view: view, spec: spec)
    }

    /// Stop pending queries and all work for the applied View.
    func close() {
        generation &+= 1
        pending?.cancel()
        pending = nil
        epoch.cancel()
        epoch = Epoch()
    }

    private func replace(_ next: Applied) {
        epoch.cancel()
        epoch = Epoch()
        applied = next
    }

    // MARK: Filter editors

    /// The rows a column's filter editor describes; see `FilterFacet`.
    func facet(_ column: UInt32) async throws -> GewuView {
        let facet = FilterFacet(spec: applied.spec, column: column)
        if facet.usesAppliedView { return applied.view }
        let dataset = dataset
        return try await scan(epoch.facets, key: column, slot: .facet(column), retain: false) {
            try facet.resolve(dataset: dataset, cancellation: $0)
        }
    }

    func filterChoices(_ column: UInt32) async throws -> FilterChoices {
        return try await cached(epoch.filterChoices, key: column, slot: .choices(column)) { signal in
            let view = try await self.facet(column)
            return try await self.run {
                try view.filterChoices(column: column, cancellation: signal)
            }
        }
    }

    func filterDistribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> RangeHistogram? {
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        return try await cached(epoch.filterDistributions, key: key, slot: .filterDistribution(column)) { signal in
            let view = try await self.facet(column)
            return try await self.run {
                try view.filterDistribution(column: column, bins: bins, scale: scale, cancellation: signal)
            }
        }
    }

    // MARK: Analysis of the applied View

    func countNulls(_ column: UInt32) async throws -> UInt64 {
        let view = applied.view
        return try await scan(epoch.nullCounts, key: column, slot: .nulls(column)) {
            try view.countNulls(column: column, cancellation: $0)
        }
    }

    func statistics(_ column: UInt32) async throws -> NumericStatistics {
        let view = applied.view
        return try await scan(epoch.statistics, key: column, slot: .statistics(column)) {
            try view.analyzeStatistics(column: column, cancellation: $0)
        }
    }

    func distribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> Histogram {
        let view = applied.view
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        return try await scan(epoch.distributions, key: key, slot: .distribution(column)) {
            try view.analyzeDistribution(column: column, bins: bins, scale: scale, cancellation: $0)
        }
    }

    func values(_ column: UInt32) async throws -> ValueSummary {
        let view = applied.view
        return try await scan(epoch.values, key: column, slot: .values(column)) {
            try view.analyzeValues(column: column, cancellation: $0)
        }
    }

    func lengths(_ column: UInt32, bins: UInt32) async throws -> LengthSummary {
        let view = applied.view
        let key = LengthKey(column: column, bins: bins)
        return try await scan(epoch.lengths, key: key, slot: .lengths(column)) {
            try view.analyzeLengths(column: column, bins: bins, cancellation: $0)
        }
    }

    func scatter(
        x: UInt32?, y: UInt32, limit: UInt32, xScale: AxisScale, yScale: AxisScale
    ) async throws -> ScatterResult {
        let view = applied.view
        let key = ScatterKey(x: x, y: y, limit: limit, xScale: xScale, yScale: yScale)
        return try await scan(epoch.scatter, key: key, slot: .scatter) {
            try view.analyzeScatter(
                xColumn: x, yColumn: y, limit: limit, xScale: xScale, yScale: yScale, cancellation: $0)
        }
    }

    private func scan<Key: Hashable, Value: Sendable>(
        _ cache: ResultCache<Key, Value>, key: Key, slot: Slot, retain: Bool = true,
        _ compute: @escaping @Sendable (Cancellation) throws -> Value
    ) async throws -> Value {
        try await cached(cache, key: key, slot: slot, retain: retain) { signal in
            try await self.run { try compute(signal) }
        }
    }

    /// Synchronous FFI scans share one worker. Cancelled queued operations never call Core.
    private func run<Value: Sendable>(_ compute: @escaping @Sendable () throws -> Value) async throws -> Value {
        let operation = ViewerScan(compute)
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            let result: Value = try await withCheckedThrowingContinuation { continuation in
                operation.completionBlock = { [weak operation] in
                    continuation.resume(with: operation?.result ?? .failure(CancellationError()))
                }
                analysisWorker.addOperation(operation)
            }
            try Task.checkCancellation()
            return result
        } onCancel: {
            operation.cancel()
        }
    }

    /// Share matching requests, retain only the most recent parameters per slot,
    /// and discard failed or superseded work. Facets share only in-flight computation.
    private func cached<Key: Hashable, Value: Sendable>(
        _ cache: ResultCache<Key, Value>, key: Key, slot: Slot, retain: Bool = true,
        _ compute: @escaping @MainActor (Cancellation) async throws -> Value
    ) async throws -> Value {
        try Task.checkCancellation()
        let epoch = epoch
        let task: Task<Value, Error>
        if let existing = cache.tasks[key] {
            task = existing
            epoch.touch(slot)
        } else {
            epoch.discard(slot)
            let signal = Cancellation()
            task = Task {
                try await withTaskCancellationHandler {
                    try Task.checkCancellation()
                    return try await compute(signal)
                } onCancel: {
                    signal.cancel()
                }
            }
            cache.tasks[key] = task
            epoch.insert(slot) { cancel in
                cache.tasks[key] = nil
                if cancel { task.cancel() }
            }
        }
        let value: Value
        do {
            value = try await task.value
            guard epoch === self.epoch, !task.isCancelled else { throw CancellationError() }
            if !retain, cache.tasks[key] == task { epoch.discard(slot, cancel: false) }
        } catch {
            if cache.tasks[key] == task { epoch.discard(slot) }
            throw error
        }
        // A panel cancelling its wait must not evict work shared with another panel.
        try Task.checkCancellation()
        return value
    }
}

/// OperationQueue owns execution and invokes completion even when cancelled before starting.
/// The lock protects the result read by its completion callback on a different thread.
private final class ViewerScan<Value: Sendable>: Operation, @unchecked Sendable {
    private let compute: @Sendable () throws -> Value
    private let lock = NSLock()
    private var output: Result<Value, Error> = .failure(CancellationError())

    init(_ compute: @escaping @Sendable () throws -> Value) {
        self.compute = compute
    }

    override func main() {
        guard !isCancelled else { return }
        let result = Result { try compute() }
        lock.lock()
        output = result
        lock.unlock()
    }

    var result: Result<Value, Error> {
        lock.lock()
        defer { lock.unlock() }
        return output
    }
}

/// The rows a column's filter editor describes: the applied filters on the other
/// columns, without sorting. The column's own filter never hides its alternatives.
struct FilterFacet {
    let spec: QuerySpec
    let column: UInt32

    /// Without its own filter, the column's facet is the applied view itself.
    var usesAppliedView: Bool {
        !spec.columns.contains { $0.column == column && $0.filter != nil }
    }

    /// May run a query over the whole dataset; call off the main actor.
    func resolve(dataset: GewuDataset, cancellation: Cancellation) throws -> GewuView {
        let others = spec.columns.compactMap { entry -> ColSpec? in
            guard entry.column != column, let filter = entry.filter else { return nil }
            return ColSpec(column: entry.column, sort: nil, filter: filter)
        }
        return try dataset.query(spec: QuerySpec(columns: others), cancellation: cancellation)
    }
}
