import SwiftUI

/// The popup keeps one facet snapshot and one candidate page. Searches share a
/// serial worker, so an obsolete scan cannot race a newer page or rebuild the facet.
@MainActor
final class TextChoicesModel: ObservableObject {
    @Published private(set) var page: TextChoicePage?
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private let session = TextChoiceSession()
    private var current: Cancellation?

    func load(
        document: GewuSession, column: UInt32, querySpec: QuerySpec,
        query: String, after: TextChoiceCursor?
    ) async {
        guard !Task.isCancelled else { return }
        current?.cancel()
        let cancellation = Cancellation()
        current = cancellation
        loading = true
        error = nil
        page = nil
        defer { if current === cancellation { loading = false } }
        do {
            try await Task.sleep(for: .milliseconds(200))
            let result = try await withTaskCancellationHandler {
                try await session.page(
                    document: document, column: column, querySpec: querySpec,
                    query: query, after: after, cancellation: cancellation)
            } onCancel: {
                cancellation.cancel()
            }
            guard !Task.isCancelled, current === cancellation else { return }
            page = result
        } catch {
            guard !Task.isCancelled, current === cancellation else { return }
            if case GewuError.Cancelled = error { return }
            self.error = error.userMessage
        }
    }

    func pause() {
        current?.cancel()
        current = nil
        loading = false
        page = nil
        error = nil
    }

    func cancel() {
        current?.cancel()
        current = nil
        session.cancel()
        loading = false
    }
}

private final class TextChoiceSession: @unchecked Sendable {
    private let worker = DispatchQueue(label: "gewu.filter.text-choices", qos: .userInitiated)
    private let lock = NSLock()
    private var closed = false
    private var job: TextChoices?

    deinit { cancel() }

    func cancel() {
        lock.lock()
        closed = true
        let job = job
        self.job = nil
        lock.unlock()
        job?.cancel()
    }

    private func prepared(document: GewuSession, column: UInt32, querySpec: QuerySpec, cancellation: Cancellation)
        throws -> TextChoices
    {
        lock.lock()
        let existing = job
        let wasClosed = closed
        lock.unlock()
        if wasClosed { throw CancellationError() }
        if let existing { return existing }
        let prepared = try document.prepareTextChoices(column: column, spec: querySpec, cancellation: cancellation)
        lock.lock()
        let closed = closed
        if !closed {
            job = prepared
        }
        lock.unlock()
        if closed {
            prepared.cancel()
            throw CancellationError()
        }
        return prepared
    }

    func page(
        document: GewuSession, column: UInt32, querySpec: QuerySpec,
        query: String, after: TextChoiceCursor?, cancellation: Cancellation
    ) async throws -> TextChoicePage {
        try await withCheckedThrowingContinuation { continuation in
            worker.async {
                do {
                    let job = try self.prepared(
                        document: document, column: column, querySpec: querySpec, cancellation: cancellation)
                    let page = try job.page(
                        query: query, after: after, limit: 100,
                        cancellation: cancellation)
                    continuation.resume(returning: page)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }
}
