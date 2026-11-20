import Synchronization

/// Shares analysis results among the consumers of one ``RowSet``.
///
/// Concurrent requests for the same key share one computation. A consumer
/// that stops waiting does not stop it for the others; only when every
/// waiter has gone does the computation get cancelled, which keeps it out of
/// Rust if it has not started yet. Completed results stay for later
/// requests, bounded by `limit`. Requests for the same ``AnalysisKey/slot``
/// with new parameters replace the old result, so a panel changing its bin
/// count does not accumulate stale histograms.
final class AnalysisCache: Sendable {
    private struct Pending {
        let task: Task<any Sendable, any Error>
        /// Distinguishes this computation from a later one for the same key.
        let generation: Int
        /// Waiters that have neither finished nor been cancelled.
        var waiters: Set<Int>
    }

    private struct State {
        var nextWaiter = 0
        var pending: [AnalysisKey: Pending] = [:]
        var completed: [AnalysisKey: any Sendable] = [:]
        /// Completed keys, least recently used first.
        var recent: [AnalysisKey] = []
    }

    private enum Lookup {
        case done(any Sendable)
        case wait(Task<any Sendable, any Error>, waiter: Int)
    }

    private let limit: Int
    private let state = Mutex(State())

    init(limit: Int = 32) {
        self.limit = limit
    }

    func value<Value: Sendable>(
        for key: AnalysisKey, compute: @escaping @Sendable () async throws -> Value
    ) async throws -> Value {
        try Task.checkCancellation()
        let lookup: Lookup = state.withLock { state in
            if let value = state.completed[key] {
                state.recent.removeAll { $0 == key }
                state.recent.append(key)
                return .done(value)
            }
            let waiter = state.nextWaiter
            state.nextWaiter &+= 1
            if var pending = state.pending[key] {
                pending.waiters.insert(waiter)
                state.pending[key] = pending
                return .wait(pending.task, waiter: waiter)
            }
            let generation = waiter
            let task = Task<any Sendable, any Error> { [weak self] in
                let value = try await compute()
                self?.store(value, for: key, generation: generation)
                return value
            }
            state.pending[key] = Pending(task: task, generation: generation, waiters: [waiter])
            return .wait(task, waiter: waiter)
        }
        switch lookup {
        case .done(let value):
            return value as! Value
        case .wait(let task, let waiter):
            defer { leave(key, task: task, waiter: waiter, cancelIfLast: false) }
            let value = try await withTaskCancellationHandler {
                try await task.value
            } onCancel: {
                self.leave(key, task: task, waiter: waiter, cancelIfLast: true)
            }
            try Task.checkCancellation()
            return value as! Value
        }
    }

    /// Keep a completed value. A computation cancelled because nobody waited
    /// may still finish; its value is kept, but it never ends a newer computation.
    private func store(_ value: any Sendable, for key: AnalysisKey, generation: Int) {
        state.withLock { state in
            if state.pending[key]?.generation == generation { state.pending[key] = nil }
            for old in state.completed.keys where old != key && old.slot == key.slot {
                state.completed[old] = nil
                state.recent.removeAll { $0 == old }
            }
            state.completed[key] = value
            state.recent.removeAll { $0 == key }
            state.recent.append(key)
            while state.recent.count > limit {
                state.completed[state.recent.removeFirst()] = nil
            }
        }
    }

    /// A waiter finished or was cancelled. When a cancelled waiter was the
    /// last one, cancel the computation; failed computations are not kept.
    private func leave(_ key: AnalysisKey, task: Task<any Sendable, any Error>, waiter: Int, cancelIfLast: Bool) {
        let cancel = state.withLock { state -> Bool in
            guard var pending = state.pending[key], pending.task == task else { return false }
            pending.waiters.remove(waiter)
            guard pending.waiters.isEmpty else {
                state.pending[key] = pending
                return false
            }
            state.pending[key] = nil
            return cancelIfLast
        }
        if cancel { task.cancel() }
    }
}

/// One analysis of one ``RowSet``, with its parameters.
enum AnalysisKey: Hashable, Sendable {
    case nullCount(ColumnID)
    case statistics(ColumnID)
    case distribution(ColumnID, bins: UInt32, scale: AxisScale)
    case values(ColumnID)
    case lengths(ColumnID, bins: UInt32)
    case scatter(x: ColumnID?, y: ColumnID, limit: UInt32, xScale: AxisScale, yScale: AxisScale)
    case filterOptions(ColumnID)
    case filterDistribution(ColumnID, bins: UInt32, scale: AxisScale)

    /// Keys with the same slot differ only in parameters; a new result replaces the old one.
    var slot: Slot {
        switch self {
        case .nullCount(let column): .nullCount(column)
        case .statistics(let column): .statistics(column)
        case .distribution(let column, _, _): .distribution(column)
        case .values(let column): .values(column)
        case .lengths(let column, _): .lengths(column)
        case .scatter: .scatter
        case .filterOptions(let column): .filterOptions(column)
        case .filterDistribution(let column, _, _): .filterDistribution(column)
        }
    }

    enum Slot: Hashable {
        case nullCount(ColumnID)
        case statistics(ColumnID)
        case distribution(ColumnID)
        case values(ColumnID)
        case lengths(ColumnID)
        case scatter
        case filterOptions(ColumnID)
        case filterDistribution(ColumnID)
    }
}
