import Foundation

/// One window's query state: the applied View, the spec it came from, and the
/// results computed for that View.
///
/// Results are cached per applied View and shared by every panel. Replacing the
/// View starts a new epoch: work still running for the old View is cancelled and
/// its results are released. Reordering columns keeps the epoch, because no cached
/// result depends on column order. Views stay immutable and can outlive their
/// epoch, so export and copy capture a View directly instead of going through here.
@MainActor
final class Viewer {
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

    enum ResultKey: Hashable {
        case facet(UInt32)
        case filterChoices(UInt32)
        case filterDistribution(UInt32, bins: UInt32, scale: AxisScale)
        case nullCount(UInt32)
        case statistics(UInt32)
        case distribution(UInt32, bins: UInt32, scale: AxisScale)
        case values(UInt32)
        case lengths(UInt32, bins: UInt32)
        case scatter(x: UInt32?, y: UInt32, limit: UInt32, xScale: AxisScale, yScale: AxisScale)
    }

    /// Work and results that belong to one applied View.
    private final class Epoch {
        let cancellation = Cancellation()
        var results: [ResultKey: Any] = [:]
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

    init(view: GewuView, spec: QuerySpec) {
        applied = Applied(view: view, spec: spec)
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
        let source = applied.view
        let worker = worker
        do {
            if debounce { try await Task.sleep(for: .milliseconds(150)) }
            try Task.checkCancellation()
            guard mine == generation else { return .superseded }
            let view: GewuView = try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        do {
                            continuation.resume(returning: try source.query(spec: spec, cancellation: signal))
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
        epoch.cancellation.cancel()
        epoch = Epoch()
    }

    private func replace(_ next: Applied) {
        epoch.cancellation.cancel()
        epoch = Epoch()
        applied = next
    }

    // MARK: Filter editors

    /// The rows a column's filter editor describes; see `FilterFacet`.
    func facet(_ column: UInt32) async throws -> GewuView {
        let facet = FilterFacet(view: applied.view, spec: applied.spec, column: column)
        if facet.usesAppliedView { return applied.view }
        return try await cached(.facet(column)) { try facet.resolve(cancellation: $0) }
    }

    func filterChoices(_ column: UInt32) async throws -> FilterChoices {
        let view = try await facet(column)
        return try await cached(.filterChoices(column)) {
            try view.filterChoices(column: column, cancellation: $0)
        }
    }

    func filterDistribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> RangeHistogram? {
        let view = try await facet(column)
        return try await cached(.filterDistribution(column, bins: bins, scale: scale)) {
            try view.filterDistribution(column: column, bins: bins, scale: scale, cancellation: $0)
        }
    }

    // MARK: Analysis of the applied View

    func countNulls(_ column: UInt32) async throws -> UInt64 {
        let view = applied.view
        return try await cached(.nullCount(column)) { try view.countNulls(column: column, cancellation: $0) }
    }

    func statistics(_ column: UInt32) async throws -> NumericStatistics {
        let view = applied.view
        return try await cached(.statistics(column)) {
            try view.analyzeStatistics(column: column, cancellation: $0)
        }
    }

    func distribution(_ column: UInt32, bins: UInt32, scale: AxisScale) async throws -> Histogram {
        let view = applied.view
        return try await cached(.distribution(column, bins: bins, scale: scale)) {
            try view.analyzeDistribution(column: column, bins: bins, scale: scale, cancellation: $0)
        }
    }

    func values(_ column: UInt32) async throws -> ValueSummary {
        let view = applied.view
        return try await cached(.values(column)) { try view.analyzeValues(column: column, cancellation: $0) }
    }

    func lengths(_ column: UInt32, bins: UInt32) async throws -> LengthSummary {
        let view = applied.view
        return try await cached(.lengths(column, bins: bins)) {
            try view.analyzeLengths(column: column, bins: bins, cancellation: $0)
        }
    }

    func scatter(
        x: UInt32?, y: UInt32, limit: UInt32, xScale: AxisScale, yScale: AxisScale
    ) async throws -> ScatterResult {
        let view = applied.view
        return try await cached(.scatter(x: x, y: y, limit: limit, xScale: xScale, yScale: yScale)) {
            try view.analyzeScatter(
                xColumn: x, yColumn: y, limit: limit, xScale: xScale, yScale: yScale, cancellation: $0)
        }
    }

    /// Compute a result once per epoch. Concurrent requests share one computation,
    /// which runs off the main actor and stops when the epoch ends. Failures are
    /// not cached, and a result that finishes after its epoch ended is reported as
    /// cancelled rather than returned.
    private func cached<T: Sendable>(
        _ key: ResultKey, _ compute: @escaping @Sendable (Cancellation) throws -> T
    ) async throws -> T {
        let epoch = epoch
        let task: Task<T, Error>
        if let existing = epoch.results[key] as? Task<T, Error> {
            task = existing
        } else {
            let signal = epoch.cancellation
            task = Task.detached(priority: .userInitiated) { try compute(signal) }
            epoch.results[key] = task
        }
        do {
            let value = try await task.value
            guard epoch === self.epoch else { throw CancellationError() }
            return value
        } catch {
            if let stored = epoch.results[key] as? Task<T, Error>, stored == task {
                epoch.results[key] = nil
            }
            throw error
        }
    }
}

/// The rows a column's filter editor describes: the applied filters on the other
/// columns, without sorting. The column's own filter never hides its alternatives.
struct FilterFacet {
    let view: GewuView
    let spec: QuerySpec
    let column: UInt32

    /// Without its own filter, the column's facet is the applied view itself.
    var usesAppliedView: Bool {
        !spec.columns.contains { $0.column == column && $0.filter != nil }
    }

    /// May run a query over the whole dataset; call off the main actor.
    func resolve(cancellation: Cancellation) throws -> GewuView {
        if usesAppliedView { return view }
        let others = spec.columns.compactMap { entry -> ColSpec? in
            guard entry.column != column, let filter = entry.filter else { return nil }
            return ColSpec(column: entry.column, sort: nil, filter: filter)
        }
        return try view.query(spec: QuerySpec(columns: others), cancellation: cancellation)
    }
}

extension Error {
    /// Cancellation from Swift or from Core; never shown to the user.
    var isCancellation: Bool {
        if self is CancellationError { return true }
        if let error = self as? GewuError, case .Cancelled = error { return true }
        return false
    }
}
