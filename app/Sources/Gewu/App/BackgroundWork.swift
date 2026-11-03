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
