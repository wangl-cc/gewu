import Combine
import Foundation

/// Presentation and bounded work for cell Find in one captured table view.
/// Only the current cell, one directional page and the visible highlight block
/// are retained in Swift.
@MainActor
final class FindModel: ObservableObject {
    @Published var query = ""
    @Published private(set) var currentMatch: CellMatch?
    @Published private(set) var highlight: FindHighlight?
    @Published private(set) var isFinding = false
    @Published private(set) var hasMatches: Bool?
    @Published private(set) var revision: UInt64 = 0
    @Published private(set) var highlightRevision: UInt64 = 0

    var onNavigate: ((CellMatch) -> Void)?
    var onError: ((Error, String) -> Void)?

    private static let pageSize: UInt32 = 32
    private static let maxHighlightRows = findLimits().maxHighlightRows

    private var session: FindSession?
    private var columns: [UInt32] = []
    private var request: UInt64 = 0
    private var navigationRequest: UInt64 = 0
    private var highlightRequest: UInt64 = 0
    private let gate = RequestGate()
    private let navigationGate = RequestGate()
    private let highlightGate = RequestGate()
    private let worker = DispatchQueue(label: "gewu.document.find", qos: .userInitiated)
    private var navigationTask: Task<Void, Never>?
    private var highlightTask: Task<Void, Never>?
    private var navigationOperation: FindOperation?
    private var highlightOperation: FindOperation?
    private var pendingHighlight: (start: UInt64, count: UInt32)?
    private var completedHighlight: (start: UInt64, count: UInt32)?
    /// The last directional page, ascending in display order, and the current
    /// match's position in it. Stepping inside the page is synchronous, so
    /// repeated Next/Previous presses are never dropped while a page is cached.
    private var page: [CellMatch] = []
    private var pageIndex: Int?
    /// Net navigation requested by the user. Positive is forward, negative is
    /// backward; each completed fetch consumes one step.
    private var pendingSteps = 0
    private var fetching = false

    var activeQuery: String? {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return session == nil || trimmed.isEmpty ? nil : trimmed
    }

    var canNavigate: Bool { session != nil && hasMatches != false }

    var status: String? {
        guard session != nil else { return nil }
        if hasMatches == false { return "0 matches" }
        if let match = currentMatch { return "match at row \(formatCount(match.displayRow + 1))" }
        return isFinding ? "Finding…" : "⏎ next match · ⇧⏎ previous"
    }

    /// Capture identity before queueing work. Replacing a query cancels only the
    /// old session; an obsolete delayed task never calls cancel on the new one.
    /// `cursor` keeps the current cell when only the captured column order changed.
    func update(
        document: GewuDocument, version: UInt64, columns: [UInt32], cursor: CellMatch? = nil,
        debounce: Bool = false
    ) {
        invalidate()
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let session: FindSession
        do {
            session = try document.prepareFind(version: version, query: trimmed, columns: columns)
        } catch {
            onError?(error, "Find error: ")
            return
        }
        request &+= 1
        let mine = request
        gate.set(mine)
        self.session = session
        self.columns = columns
        revision = mine
        if let cursor {
            // The view and query are unchanged; only the captured column order
            // moved. Keep the cell and let the next highlight refresh repaint it.
            currentMatch = cursor
            hasMatches = true
            isFinding = false
        } else {
            isFinding = true
            hasMatches = nil
            pendingSteps = 1
            pump(session: session, request: mine, debounce: debounce)
        }
    }

    func next(backwards: Bool = false) {
        guard let session else { return }
        pendingSteps += backwards ? -1 : 1
        pump(session: session, request: request, debounce: false)
    }

    func invalidate() {
        request &+= 1
        gate.set(request)
        navigationRequest &+= 1
        navigationGate.set(navigationRequest)
        highlightRequest &+= 1
        highlightGate.set(highlightRequest)
        navigationTask?.cancel()
        highlightTask?.cancel()
        navigationOperation?.cancel()
        highlightOperation?.cancel()
        navigationOperation = nil
        highlightOperation = nil
        session?.cancel()
        session = nil
        columns = []
        currentMatch = nil
        isFinding = false
        hasMatches = nil
        page = []
        pageIndex = nil
        pendingSteps = 0
        fetching = false
        pendingHighlight = nil
        completedHighlight = nil
        highlight = nil
        highlightRevision &+= 1
        revision = request
    }

    func stop() { invalidate() }

    /// Fetch only flags for the visible rows. Formatted RowCache blocks are not
    /// invalidated when this query or its highlight result changes.
    func requestHighlight(start: UInt64, count: UInt32) {
        guard let session, count > 0 else { return }
        let count = min(count, Self.maxHighlightRows)
        if pendingHighlight?.start == start, pendingHighlight?.count == count { return }
        if completedHighlight?.start == start, completedHighlight?.count == count { return }
        pendingHighlight = (start, count)
        completedHighlight = nil
        highlightRequest &+= 1
        let mine = highlightRequest
        highlightGate.set(mine)
        let findRequest = request
        let worker = worker
        let gate = gate
        let highlightGate = highlightGate
        highlightOperation?.cancel()
        let operation = session.operation()
        highlightOperation = operation
        highlightTask?.cancel()
        highlightTask = Task { [weak self] in
            guard !Task.isCancelled, let self, self.request == findRequest, self.highlightRequest == mine else {
                return
            }
            do {
                let result: FindHighlight = try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        guard gate.isCurrent(findRequest), highlightGate.isCurrent(mine) else {
                            continuation.resume(throwing: CancellationError())
                            return
                        }
                        do {
                            continuation.resume(
                                returning: try session.highlightWithOperation(
                                    start: start, count: count, operation: operation))
                        } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                guard !Task.isCancelled, self.request == findRequest, self.highlightRequest == mine else {
                    return
                }
                self.pendingHighlight = nil
                self.completedHighlight = (start, count)
                self.highlight = result
                self.highlightRevision &+= 1
            } catch {
                guard !Task.isCancelled, self.request == findRequest, self.highlightRequest == mine else {
                    return
                }
                self.pendingHighlight = nil
                if case GewuError.Cancelled = error { return }
                self.onError?(error, "Find highlight error: ")
            }
        }
    }

    func isMatch(row: Int, column: Int) -> Bool {
        guard let highlight, row >= Int(highlight.start), row < Int(highlight.start) + Int(highlight.nRows),
            let scopeColumn = columns.firstIndex(of: UInt32(column))
        else { return false }
        let index = (row - Int(highlight.start)) * columns.count + scopeColumn
        return index < highlight.matched.count && highlight.matched[index]
    }

    /// Consume queued steps from the cached page, or start one fetch when the
    /// page is exhausted. Every loop iteration either moves one step or starts
    /// exactly one fetch, so a burst of presses cannot be coalesced away.
    private func pump(session: FindSession, request findRequest: UInt64, debounce: Bool) {
        while pendingSteps != 0, !fetching {
            let backwards = pendingSteps < 0
            guard let index = pageIndex, let next = steppedIndex(from: index, backwards: backwards) else {
                fetch(session: session, request: findRequest, backwards: backwards, debounce: debounce)
                return
            }
            pageIndex = next
            pendingSteps += backwards ? 1 : -1
            publish(page[next])
        }
    }

    private func steppedIndex(from index: Int, backwards: Bool) -> Int? {
        let candidate = backwards ? index - 1 : index + 1
        return candidate >= 0 && candidate < page.count ? candidate : nil
    }

    /// Fetch one page relative to the current match. A fetch spans the page
    /// boundary, so navigation never scans the same range twice.
    private func fetch(session: FindSession, request findRequest: UInt64, backwards: Bool, debounce: Bool) {
        guard !fetching else { return }
        fetching = true
        isFinding = true
        navigationRequest &+= 1
        let mine = navigationRequest
        navigationGate.set(mine)
        let cursor = currentMatch
        let worker = worker
        let gate = gate
        let navigationGate = navigationGate
        navigationOperation?.cancel()
        let operation = session.operation()
        navigationOperation = operation
        navigationTask?.cancel()
        let limit = Self.pageSize
        navigationTask = Task { [weak self] in
            do {
                if debounce { try await Task.sleep(for: .milliseconds(150)) }
                let page: [CellMatch] = try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        guard gate.isCurrent(findRequest), navigationGate.isCurrent(mine) else {
                            continuation.resume(throwing: CancellationError())
                            return
                        }
                        do {
                            var page = try readFindPage(
                                session: session, cursor: cursor, backwards: backwards, limit: limit,
                                operation: operation)
                            // A boundary wraps only after an actual cursor. The first
                            // empty scan establishes zero matches without scanning twice.
                            if page.isEmpty, cursor != nil {
                                page = try readFindPage(
                                    session: session, cursor: nil, backwards: backwards, limit: limit,
                                    operation: operation)
                            }
                            continuation.resume(returning: page)
                        } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                guard !Task.isCancelled, let self, self.request == findRequest, self.navigationRequest == mine else {
                    return
                }
                self.fetching = false
                self.apply(newPage: page, backwards: backwards)
                self.pump(session: session, request: findRequest, debounce: false)
            } catch {
                guard !Task.isCancelled, let self, self.request == findRequest, self.navigationRequest == mine else {
                    return
                }
                self.fetching = false
                self.isFinding = false
                if case GewuError.Cancelled = error { return }
                self.onError?(error, "Find error: ")
            }
        }
    }

    private func apply(newPage: [CellMatch], backwards: Bool) {
        guard pendingSteps != 0 else {
            // Forward and backward presses cancelled out while the page was loading.
            isFinding = false
            return
        }
        page = newPage
        guard !newPage.isEmpty else {
            pageIndex = nil
            pendingSteps = 0
            isFinding = false
            currentMatch = nil
            hasMatches = false
            return
        }
        let index = backwards ? newPage.count - 1 : 0
        pageIndex = index
        pendingSteps += backwards ? 1 : -1
        publish(newPage[index])
    }

    private func publish(_ match: CellMatch) {
        isFinding = false
        currentMatch = match
        hasMatches = true
        onNavigate?(match)
    }
}

/// Read one directional page on the Find worker queue.
private func readFindPage(
    session: FindSession, cursor: CellMatch?, backwards: Bool, limit: UInt32, operation: FindOperation
) throws -> [CellMatch] {
    backwards
        ? try session.previousWithOperation(before: cursor, limit: limit, operation: operation)
        : try session.nextWithOperation(after: cursor, limit: limit, operation: operation)
}
