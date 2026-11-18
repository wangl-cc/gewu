import Foundation

/// One window's query state: the applied View, the spec it came from, and the
/// results computed for that View.
///
/// Completed results are cached per applied View. Each caller owns its pending scan;
/// cancelling one caller never cancels another caller's scan. Replacing the View
/// cancels all old scans and releases their results. Reordering columns keeps the cache, because no cached
/// result depends on column order. Views stay immutable and can outlive their
/// epoch, so export and copy capture a View directly instead of going through here.
@MainActor
final class Viewer {
    let dataset: GewuDataset

    struct Applied {
        let view: GewuView
        let spec: QuerySpec
        /// The same row snapshot survives column-only reordering.
        let rowView: GewuView

        init(view: GewuView, spec: QuerySpec, rowView: GewuView? = nil) {
            self.view = view
            self.spec = spec
            self.rowView = rowView ?? view
        }
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
        var values: [Key: Value] = [:]
    }

    /// Parameters select a result within a slot; changing them replaces that slot.
    private enum Slot: Hashable {
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
        let filterChoices = ResultCache<UInt32, FilterChoices>()
        let filterDistributions = ResultCache<DistributionKey, RangeHistogram?>()
        let nullCounts = ResultCache<UInt32, UInt64>()
        let statistics = ResultCache<UInt32, NumericStatistics>()
        let distributions = ResultCache<DistributionKey, Histogram>()
        let values = ResultCache<UInt32, ValueSummary>()
        let lengths = ResultCache<LengthKey, LengthSummary>()
        let scatter = ResultCache<ScatterKey, ScatterResult>()

        private var recent: [Slot] = []
        private var removals: [Slot: () -> Void] = [:]
        private var tickets: [Slot: UUID] = [:]
        var active: [ObjectIdentifier: () -> Void] = [:]

        func touch(_ slot: Slot) {
            recent.removeAll { $0 == slot }
            recent.append(slot)
        }

        /// Bound cache slots and reserve the latest parameters without taking ownership
        /// of a caller's work. An older scan may finish for its caller but cannot replace it.
        func reserve(_ slot: Slot) -> UUID {
            discard(slot)
            let ticket = UUID()
            tickets[slot] = ticket
            touch(slot)
            if recent.count > 32 { discard(recent[0]) }
            return ticket
        }

        func insert(_ slot: Slot, ticket: UUID, remove: @escaping () -> Void) -> Bool {
            guard tickets[slot] == ticket else { return false }
            removals[slot] = remove
            return true
        }

        func discard(_ slot: Slot) {
            removals.removeValue(forKey: slot)?()
            tickets[slot] = nil
            recent.removeAll { $0 == slot }
        }

        func cancel() {
            for cancel in active.values { cancel() }
            active.removeAll()
            for remove in removals.values { remove() }
            removals.removeAll()
            tickets.removeAll()
            recent.removeAll()
        }
    }

    private(set) var applied: Applied
    private var epoch = Epoch()
    // Queries are serialized; the window owns cancellation and result publication.
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

    /// Compute a replacement without publishing it. The window also loads the
    /// target viewport before committing the new View and its presentation.
    func query(_ spec: QuerySpec, debounce: Bool = false) async throws -> GewuView {
        try Task.checkCancellation()
        let flag = CancelFlag()
        let dataset = dataset
        let worker = worker
        return try await withTaskCancellationHandler {
            if debounce { try await Task.sleep(for: .milliseconds(150)) }
            try Task.checkCancellation()
            let view: GewuView = try await withCheckedThrowingContinuation { continuation in
                worker.async {
                    guard !flag.isCancelled else {
                        continuation.resume(throwing: CancellationError())
                        return
                    }
                    do {
                        continuation.resume(returning: try dataset.query(spec: spec))
                    } catch {
                        continuation.resume(throwing: error)
                    }
                }
            }
            try Task.checkCancellation()
            return view
        } onCancel: {
            flag.cancel()
        }
    }

    func apply(_ view: GewuView, spec: QuerySpec) {
        epoch.cancel()
        epoch = Epoch()
        applied = Applied(view: view, spec: spec)
    }

    /// Show the same rows in another column order. Cached results stay valid.
    func reorder(_ columns: [UInt32], spec: QuerySpec) throws {
        let view = try applied.view.reorderColumns(order: columns)
        applied = Applied(view: view, spec: spec, rowView: applied.rowView)
    }

    /// Stop analysis work for the applied View. Query tasks belong to the window.
    func close() {
        epoch.cancel()
        epoch = Epoch()
    }

    // MARK: Filter editors

    /// The rows a column's filter editor describes; see `FilterFacet`.
    func facet(_ column: UInt32) async throws -> GewuView {
        let facet = FilterFacet(spec: applied.spec, column: column)
        try Task.checkCancellation()
        if facet.usesAppliedView { return applied.rowView }
        let dataset = dataset
        return try await run {
            try facet.resolve(dataset: dataset)
        }
    }

    func filterChoices(_ column: UInt32) async throws -> FilterChoices {
        let applied = applied
        let dataset = dataset
        let facet = FilterFacet(spec: applied.spec, column: column)
        return try await scan(epoch.filterChoices, key: column, slot: .choices(column)) {
            let view =
                try facet.usesAppliedView ? applied.rowView : facet.resolve(dataset: dataset)
            return try view.filterChoices(column: column)
        }
    }

    func filterDistribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> RangeHistogram? {
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        let applied = applied
        let dataset = dataset
        let facet = FilterFacet(spec: applied.spec, column: column)
        return try await scan(epoch.filterDistributions, key: key, slot: .filterDistribution(column)) {
            let view =
                try facet.usesAppliedView ? applied.rowView : facet.resolve(dataset: dataset)
            return try view.filterDistribution(column: column, bins: bins, scale: scale)
        }
    }

    // MARK: Analysis of the applied View

    func countNulls(_ column: UInt32) async throws -> UInt64 {
        let view = applied.view
        return try await scan(epoch.nullCounts, key: column, slot: .nulls(column)) {
            try view.countNulls(column: column)
        }
    }

    func statistics(_ column: UInt32) async throws -> NumericStatistics {
        let view = applied.view
        return try await scan(epoch.statistics, key: column, slot: .statistics(column)) {
            try view.analyzeStatistics(column: column)
        }
    }

    func distribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> Histogram {
        let view = applied.view
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        return try await scan(epoch.distributions, key: key, slot: .distribution(column)) {
            try view.analyzeDistribution(column: column, bins: bins, scale: scale)
        }
    }

    func values(_ column: UInt32) async throws -> ValueSummary {
        let view = applied.view
        return try await scan(epoch.values, key: column, slot: .values(column)) {
            try view.analyzeValues(column: column)
        }
    }

    func lengths(_ column: UInt32, bins: UInt32) async throws -> LengthSummary {
        let view = applied.view
        let key = LengthKey(column: column, bins: bins)
        return try await scan(epoch.lengths, key: key, slot: .lengths(column)) {
            try view.analyzeLengths(column: column, bins: bins)
        }
    }

    func scatter(
        x: UInt32?, y: UInt32, limit: UInt32, xScale: AxisScale, yScale: AxisScale
    ) async throws -> ScatterResult {
        let view = applied.view
        let key = ScatterKey(x: x, y: y, limit: limit, xScale: xScale, yScale: yScale)
        return try await scan(epoch.scatter, key: key, slot: .scatter) {
            try view.analyzeScatter(
                xColumn: x, yColumn: y, limit: limit, xScale: xScale, yScale: yScale)
        }
    }

    private func scan<Key: Hashable, Value: Sendable>(
        _ cache: ResultCache<Key, Value>, key: Key, slot: Slot,
        _ compute: @escaping @Sendable () throws -> Value
    ) async throws -> Value {
        try Task.checkCancellation()
        let epoch = epoch
        if let value = cache.values[key] {
            epoch.touch(slot)
            return value
        }
        let ticket = epoch.reserve(slot)
        let value = try await run(compute)
        guard epoch === self.epoch else { throw CancellationError() }
        if epoch.insert(slot, ticket: ticket, remove: { cache.values[key] = nil }) {
            cache.values[key] = .some(value)
        }
        return value
    }

    /// Synchronous FFI scans share one worker. Cancelled queued operations never call Core.
    private func run<Value: Sendable>(_ compute: @escaping @Sendable () throws -> Value) async throws
        -> Value
    {
        try Task.checkCancellation()
        let epoch = epoch
        let operation = ViewerScan(compute)
        let id = ObjectIdentifier(operation)
        epoch.active[id] = {
            operation.cancel()
        }
        defer { epoch.active[id] = nil }
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            let result: Value = try await withCheckedThrowingContinuation { continuation in
                operation.completionBlock = { [weak operation] in
                    continuation.resume(with: operation?.result ?? .failure(CancellationError()))
                }
                analysisWorker.addOperation(operation)
            }
            try Task.checkCancellation()
            guard epoch === self.epoch, !operation.isCancelled else { throw CancellationError() }
            return result
        } onCancel: {
            operation.cancel()
        }
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
    func resolve(dataset: GewuDataset) throws -> GewuView {
        let others = spec.columns.compactMap { entry -> ColSpec? in
            guard entry.column != column, let filter = entry.filter else { return nil }
            return ColSpec(column: entry.column, sort: nil, filter: filter)
        }
        return try dataset.query(spec: QuerySpec(columns: others))
    }
}
