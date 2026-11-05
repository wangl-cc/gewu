import Foundation

/// Run a synchronous scan off the UI thread and forward the owning Task's cancellation.
func runCancellable<T: Sendable>(
    cancellation: Cancellation,
    operation: @escaping @Sendable () throws -> T
) async throws -> T {
    try await withTaskCancellationHandler {
        try Task.checkCancellation()
        let result = try await Task.detached(priority: .userInitiated) {
            try operation()
        }.value
        try Task.checkCancellation()
        return result
    } onCancel: {
        cancellation.cancel()
    }
}

/// The worker reads this before starting a queued synchronous FFI call.
final class RequestGate: @unchecked Sendable {
    private let lock = NSLock()
    private var value: UInt64 = 0
    func set(_ value: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        self.value = value
    }
    func isCurrent(_ value: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return self.value == value
    }
}
