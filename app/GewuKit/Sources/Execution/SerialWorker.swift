import Dispatch

/// Runs synchronous Rust calls one at a time on a dedicated thread.
///
/// Core scans cannot be interrupted, so a worker offers the strongest
/// cancellation that is still possible: a call whose task is cancelled before
/// it reaches the front of the queue never enters Rust, and a call that has
/// started runs to completion while its caller discards the result. Long scans
/// occupy this worker's queue rather than a thread of Swift's cooperative pool.
actor SerialWorker {
    private let queue: DispatchSerialQueue

    init(label: String) {
        queue = DispatchSerialQueue(label: "io.github.wangl-cc.gewu.\(label)", qos: .userInitiated)
    }

    nonisolated var unownedExecutor: UnownedSerialExecutor {
        queue.asUnownedSerialExecutor()
    }

    /// Run `body` on this worker unless the calling task is already cancelled.
    /// Rust errors arrive as ``GewuKitError`` or `CancellationError`.
    func perform<Value: Sendable>(_ body: @Sendable () throws -> Value) throws -> Value {
        try Task.checkCancellation()
        return try translatingErrors(body)
    }
}

/// Map a Rust error to the error GewuKit reports.
func translatingErrors<Value>(_ body: () throws -> Value) throws -> Value {
    do {
        return try body()
    } catch let error as FfiError {
        throw error.translated
    }
}

/// The workers that serve one opened table. Each kind of work has its own
/// queue, so a long analysis never delays scrolling or a query.
struct Workers: Sendable {
    let query = SerialWorker(label: "query")
    let analysis = SerialWorker(label: "analysis")
    let read = SerialWorker(label: "read")
    let find = SerialWorker(label: "find")
}
