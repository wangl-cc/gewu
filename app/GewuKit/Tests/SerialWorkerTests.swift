import Foundation
import Synchronization
import Testing

@testable import GewuKit

@Suite struct SerialWorkerTests {
    @Test func queuedWorkCancelledBeforeItStartsNeverRuns() async throws {
        let worker = SerialWorker(label: "test")
        let gate = DispatchSemaphore(value: 0)
        let ran = Mutex(false)
        let blocker = Task { try await worker.perform { gate.wait() } }
        // Let the blocker occupy the worker before queueing behind it.
        try await Task.sleep(for: .milliseconds(50))
        let queued = Task { try await worker.perform { ran.withLock { $0 = true } } }
        try await Task.sleep(for: .milliseconds(50))
        queued.cancel()
        gate.signal()
        try await blocker.value
        await #expect(throws: CancellationError.self) { try await queued.value }
        #expect(!ran.withLock { $0 })
    }

    @Test func rustCancellationArrivesAsCancellationError() {
        #expect(FfiError.Cancelled.translated is CancellationError)
        #expect(FfiError.Internal(message: "m").translated as? GewuKitError == .internalFailure("m"))
    }
}
