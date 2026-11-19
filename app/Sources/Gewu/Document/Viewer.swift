import Foundation
import Observation

/// One window's query state: the applied filtered and ordered views, the
/// filters and sorts they came from, and the analysis results computed for
/// `filtered`.
///
/// Completed results are cached per `filtered` Epoch. Each caller owns its
/// pending scan; cancelling one caller never cancels another caller's scan.
/// Replacing `filtered` cancels all old scans and releases their results.
/// Pure sorting and column reordering/visibility changes keep the Epoch cache,
/// because no cached analysis result depends on row order or column order.
/// Views stay immutable and can outlive their epoch, so export and copy capture
/// `ordered` directly instead of going through here.
@MainActor
@Observable
final class Viewer {
    let dataset: GewuDataset

    private struct DistributionKey: Hashable {
        let column: ColumnId
        let bins: UInt32
        let scale: AxisScale
    }

    private struct LengthKey: Hashable {
        let column: ColumnId
        let bins: UInt32
    }

    private struct ScatterKey: Hashable {
        let x: ColumnId?
        let y: ColumnId
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
        case choices(ColumnId)
        case filterDistribution(ColumnId)
        case nulls(ColumnId)
        case statistics(ColumnId)
        case distribution(ColumnId)
        case values(ColumnId)
        case lengths(ColumnId)
        case scatter
    }

    /// Work and results that belong to one applied `GewuFilteredView`.
    @MainActor
    private final class Epoch {
        let filterChoices = ResultCache<ColumnId, FilterChoices>()
        let filterDistributions = ResultCache<DistributionKey, RangeHistogram?>()
        let nullCounts = ResultCache<ColumnId, UInt64>()
        let statistics = ResultCache<ColumnId, NumericStatistics>()
        let distributions = ResultCache<DistributionKey, Histogram>()
        let values = ResultCache<ColumnId, ValueSummary>()
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

    private(set) var filtered: GewuFilteredView
    private(set) var ordered: GewuOrderedView
    private(set) var appliedFilters: [ColumnFilter]
    private(set) var appliedSorts: [ColumnSort]

    @ObservationIgnored private var epoch = Epoch()
    // Queries are serialized; the window owns cancellation and result publication.
    @ObservationIgnored private let queryWorker: OperationQueue
    @ObservationIgnored private let analysisWorker: OperationQueue

    init(
        dataset: GewuDataset,
        filtered: GewuFilteredView,
        ordered: GewuOrderedView,
        filters: [ColumnFilter] = [],
        sorts: [ColumnSort] = [],
        queryWorker: OperationQueue? = nil,
        analysisWorker: OperationQueue? = nil
    ) {
        self.dataset = dataset
        self.filtered = filtered
        self.ordered = ordered
        appliedFilters = filters
        appliedSorts = sorts
        self.queryWorker = queryWorker ?? OperationQueue()
        self.queryWorker.name = "gewu.viewer.queries"
        self.queryWorker.qualityOfService = .userInitiated
        self.queryWorker.maxConcurrentOperationCount = 1
        self.analysisWorker = analysisWorker ?? OperationQueue()
        self.analysisWorker.name = "gewu.viewer.analysis"
        self.analysisWorker.qualityOfService = .userInitiated
        self.analysisWorker.maxConcurrentOperationCount = 1
    }

    /// Compute a replacement filtered view without publishing it.
    func filter(_ filters: [ColumnFilter], debounce: Bool = false) async throws -> GewuFilteredView {
        let dataset = dataset
        return try await performOffMain(debounce: debounce) {
            try dataset.filter(filters: filters)
        }
    }

    /// Compute a replacement ordered view without publishing it.
    func sort(_ sorts: [ColumnSort], on filtered: GewuFilteredView, debounce: Bool = false) async throws
        -> GewuOrderedView
    {
        try await performOffMain(debounce: debounce) {
            try filtered.sort(sorts: sorts)
        }
    }

    private func performOffMain<Value: Sendable>(
        debounce: Bool, _ compute: @escaping @Sendable () throws -> Value
    ) async throws -> Value {
        try Task.checkCancellation()
        let operation = ViewerScan(compute)
        let worker = queryWorker
        return try await withTaskCancellationHandler {
            if debounce { try await Task.sleep(for: .milliseconds(150)) }
            try Task.checkCancellation()
            let value: Value = try await withCheckedThrowingContinuation { continuation in
                operation.completionBlock = { [weak operation] in
                    continuation.resume(with: operation?.result ?? .failure(CancellationError()))
                }
                worker.addOperation(operation)
            }
            try Task.checkCancellation()
            guard !operation.isCancelled else { throw CancellationError() }
            return value
        } onCancel: {
            operation.cancel()
        }
    }

    func apply(
        filtered: GewuFilteredView,
        ordered: GewuOrderedView,
        filters: [ColumnFilter],
        sorts: [ColumnSort]
    ) {
        epoch.cancel()
        epoch = Epoch()
        self.filtered = filtered
        self.ordered = ordered
        appliedFilters = filters
        appliedSorts = sorts
    }

    /// Apply a sort change on the same filtered rows, keeping cached analysis results.
    func apply(ordered: GewuOrderedView, sorts: [ColumnSort]) {
        self.ordered = ordered
        appliedSorts = sorts
    }

    /// Stop analysis work for the applied view. Query tasks belong to the window.
    func close() {
        epoch.cancel()
        epoch = Epoch()
    }

    // MARK: Filter editors

    func filterChoices(_ column: ColumnId) async throws -> FilterChoices {
        let filtered = filtered
        return try await scan(epoch.filterChoices, key: column, slot: .choices(column)) {
            try filtered.facet(column: column).filterChoices(column: column)
        }
    }

    func filterDistribution(_ column: ColumnId, bins: UInt32, scale: AxisScale) async throws -> RangeHistogram? {
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        let filtered = filtered
        return try await scan(epoch.filterDistributions, key: key, slot: .filterDistribution(column)) {
            try filtered.facet(column: column).filterDistribution(column: column, bins: bins, scale: scale)
        }
    }

    // MARK: Analysis of the applied filtered view

    func countNulls(_ column: ColumnId) async throws -> UInt64 {
        let view = filtered
        return try await scan(epoch.nullCounts, key: column, slot: .nulls(column)) {
            try view.countNulls(column: column)
        }
    }

    func statistics(_ column: ColumnId) async throws -> NumericStatistics {
        let view = filtered
        return try await scan(epoch.statistics, key: column, slot: .statistics(column)) {
            try view.statistics(column: column)
        }
    }

    func distribution(_ column: ColumnId, bins: UInt32, scale: AxisScale) async throws -> Histogram {
        let view = filtered
        let key = DistributionKey(column: column, bins: bins, scale: scale)
        return try await scan(epoch.distributions, key: key, slot: .distribution(column)) {
            try view.distribution(column: column, bins: bins, scale: scale)
        }
    }

    func values(_ column: ColumnId) async throws -> ValueSummary {
        let view = filtered
        return try await scan(epoch.values, key: column, slot: .values(column)) {
            try view.values(column: column)
        }
    }

    func lengths(_ column: ColumnId, bins: UInt32) async throws -> LengthSummary {
        let view = filtered
        let key = LengthKey(column: column, bins: bins)
        return try await scan(epoch.lengths, key: key, slot: .lengths(column)) {
            try view.lengths(column: column, bins: bins)
        }
    }

    func scatter(
        x: ColumnId?, y: ColumnId, limit: UInt32, xScale: AxisScale, yScale: AxisScale
    ) async throws -> ScatterResult {
        let view = filtered
        let key = ScatterKey(x: x, y: y, limit: limit, xScale: xScale, yScale: yScale)
        return try await scan(epoch.scatter, key: key, slot: .scatter) {
            try view.scatter(
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
