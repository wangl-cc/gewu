import Foundation

/// Per-request cancellation flag checked before a queued synchronous FFI call starts.
final class CancelFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    func cancel() {
        lock.lock()
        cancelled = true
        lock.unlock()
    }

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
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
