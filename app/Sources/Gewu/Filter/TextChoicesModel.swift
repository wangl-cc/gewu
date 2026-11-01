import SwiftUI

/// The popup keeps one facet snapshot and one candidate page. Searches share a
/// serial worker, so an obsolete scan cannot race a newer page or rebuild the facet.
@MainActor
final class TextChoicesModel: ObservableObject {
    @Published private(set) var page: TextChoicePage?
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private let session = TextChoiceSession()

    func load(
        document: GewuDocument, column: UInt32, querySpec: QuerySpec,
        query: String, after: TextChoiceCursor?
    ) async {
        guard !Task.isCancelled else { return }
        let ticket = session.advance()
        loading = true
        error = nil
        page = nil
        defer { if session.isCurrent(ticket) { loading = false } }
        do {
            try await Task.sleep(for: .milliseconds(200))
            let result = try await session.page(
                document: document, column: column, querySpec: querySpec,
                query: query, after: after, ticket: ticket)
            guard !Task.isCancelled, session.isCurrent(ticket) else { return }
            page = result
        } catch {
            guard !Task.isCancelled, session.isCurrent(ticket) else { return }
            self.error = error.userMessage
        }
    }

    func pause() {
        _ = session.advance()
        loading = false
        page = nil
        error = nil
    }

    func cancel() {
        session.cancel()
        loading = false
    }
}

private final class TextChoiceSession: @unchecked Sendable {
    private let worker = DispatchQueue(label: "gewu.filter.text-choices", qos: .userInitiated)
    private let lock = NSLock()
    private var request: UInt64 = 0
    private var closed = false
    private var job: TextChoices?
    private var generation: UInt64 = 0

    deinit { cancel() }

    func advance() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        request &+= 1
        if let job { generation = job.invalidatePages() }
        return request
    }

    func isCurrent(_ ticket: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return !closed && ticket == request
    }

    func cancel() {
        lock.lock()
        closed = true
        let job = job
        self.job = nil
        lock.unlock()
        job?.cancel()
    }

    private func prepared(document: GewuDocument, column: UInt32, querySpec: QuerySpec) throws -> TextChoices {
        lock.lock()
        let existing = job
        lock.unlock()
        if let existing { return existing }
        // Preparation computes the facet and currently has no cancellation handle.
        // If closed meanwhile, cancel the returned job before starting any page.
        let prepared = try document.prepareTextChoices(column: column, spec: querySpec)
        lock.lock()
        let closed = closed
        if !closed {
            job = prepared
            generation = prepared.invalidatePages()
        }
        lock.unlock()
        if closed {
            prepared.cancel()
            throw CancellationError()
        }
        return prepared
    }

    private func pageGeneration(for ticket: UInt64) throws -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        guard !closed, ticket == request else { throw CancellationError() }
        return generation
    }

    func page(
        document: GewuDocument, column: UInt32, querySpec: QuerySpec,
        query: String, after: TextChoiceCursor?, ticket: UInt64
    ) async throws -> TextChoicePage {
        try await withCheckedThrowingContinuation { continuation in
            worker.async {
                do {
                    guard self.isCurrent(ticket) else { throw CancellationError() }
                    let job = try self.prepared(document: document, column: column, querySpec: querySpec)
                    let generation = try self.pageGeneration(for: ticket)
                    let page = try job.pageForGeneration(
                        query: query, after: after, limit: 100,
                        generation: generation)
                    guard self.isCurrent(ticket) else { throw CancellationError() }
                    continuation.resume(returning: page)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }
}
