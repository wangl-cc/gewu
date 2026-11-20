import Foundation
import Observation

/// Presentation and bounded work for cell Find in one captured table view.
/// Only the current cell, one directional page and a bounded cache of
/// per-column highlight bitmaps are retained in Swift.
@MainActor
@Observable
final class FindModel {
    var query = ""
    private(set) var currentMatch: CellMatch?
    private(set) var isFinding = false
    private(set) var hasMatches: Bool?
    private(set) var revision: UInt64 = 0
    private(set) var highlightRevision: UInt64 = 0
    private var session: FindSession?

    @ObservationIgnored var onNavigate: ((CellMatch) -> Void)?
    @ObservationIgnored var onError: ((Error, String) -> Void)?

    private static let pageSize: UInt32 = 32

    /// One column over a block of `rowCount` display rows: the unit of the
    /// highlight cache, independent of how the table caches its text.
    private struct HighlightKey: Hashable, Sendable {
        static let rowCount = 64

        let block: Int
        let column: ColumnId

        init(block: Int, column: ColumnId) {
            self.block = block
            self.column = column
        }

        init(row: Int, column: ColumnId) { self.init(block: row / Self.rowCount, column: column) }
    }

    /// One Core request: consecutive blocks and every column missing in any of them.
    private struct HighlightRead: Sendable {
        let blocks: ClosedRange<Int>
        let columns: [ColumnId]
    }

    /// Blocks per Core request, so each column's predicate compiles once per request.
    private static let blocksPerRead = max(1, Int(findLimits().maxHighlightRows) / HighlightKey.rowCount)

    @ObservationIgnored private var columns: [ColumnId] = []
    @ObservationIgnored private var request: UInt64 = 0
    @ObservationIgnored private var navigationRequest: UInt64 = 0
    @ObservationIgnored private let gate = RequestGate()
    @ObservationIgnored private let navigationGate = RequestGate()
    @ObservationIgnored private let worker = DispatchQueue(label: "gewu.document.find", qos: .userInitiated)
    @ObservationIgnored private var navigationTask: Task<Void, Never>?
    @ObservationIgnored private var highlightTask: Task<Void, Never>?
    /// The latest on-screen rows and columns. A read in flight is never cancelled
    /// for a newer demand: its blocks stay useful, and the next read follows the
    /// latest demand when it completes.
    @ObservationIgnored private var highlightDemand: (rows: Range<Int>, columns: [ColumnId])?
    /// Match bitmaps, one bit per row. Requested only for the columns on screen.
    @ObservationIgnored private let highlights = LRUCache<HighlightKey, Data>(
        costLimit: 256 * 1024, entryCostLimit: 1024)
    /// Blocks whose read failed for this session; they are not retried.
    @ObservationIgnored private var failedHighlights: Set<HighlightKey> = []
    /// The last directional page, ascending in display order, and the current
    /// match's position in it. Stepping inside the page is synchronous, so
    /// repeated Next/Previous presses are never dropped while a page is cached.
    @ObservationIgnored private var page: [CellMatch] = []
    @ObservationIgnored private var pageIndex: Int?
    /// Net navigation requested by the user. Positive is forward, negative is
    /// backward; each completed fetch consumes one step.
    @ObservationIgnored private var pendingSteps = 0
    @ObservationIgnored private var fetching = false

    var activeQuery: String? {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return session == nil || trimmed.isEmpty ? nil : trimmed
    }

    var canNavigate: Bool { session != nil && hasMatches != false }

    var status: String? {
        guard session != nil else { return nil }
        if hasMatches == false { return "0 matches" }
        if let match = currentMatch {
            return "Row \(formatCount(match.displayRow + 1))"
        }
        return isFinding ? "Finding…" : "Press Return"
    }

    /// Capture identity before queueing work. Replacing a query cancels only the
    /// old session; an obsolete delayed task never calls cancel on the new one.
    /// `cursor` keeps the current cell when only the captured column order changed.
    func update(
        ordered: GewuOrderedView, columns: [ColumnId], cursor: CellMatch? = nil,
        debounce: Bool = false
    ) {
        invalidate()
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let session: FindSession
        do {
            session = try ordered.find(query: trimmed, columns: columns)
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
        let hadSession = session != nil
        request &+= 1
        gate.set(request)
        navigationRequest &+= 1
        navigationGate.set(navigationRequest)
        navigationTask?.cancel()
        highlightTask?.cancel()
        navigationTask = nil
        highlightTask = nil
        if session != nil { session = nil }
        columns = []
        if currentMatch != nil { currentMatch = nil }
        if isFinding { isFinding = false }
        if hasMatches != nil { hasMatches = nil }
        page = []
        pageIndex = nil
        pendingSteps = 0
        fetching = false
        highlightDemand = nil
        failedHighlights.removeAll()
        if highlights.count > 0 {
            highlights.removeAll()
            highlightRevision &+= 1
        }
        if hadSession { revision = request }
    }

    func stop() { invalidate() }

    var hasHighlights: Bool { highlights.count > 0 }

    /// Fetch flags for the visible rows of the given on-screen columns, reading
    /// only blocks that are not cached yet. Cached table segments are not
    /// invalidated when this query or its highlight result changes.
    func requestHighlight(rows: Range<Int>, columns: [ColumnId]) {
        guard session != nil, !rows.isEmpty, rows.lowerBound >= 0 else { return }
        let scope = Set(self.columns)
        highlightDemand = (rows, columns.filter(scope.contains))
        loadHighlights()
    }

    func isMatch(row: Int, column: ColumnId) -> Bool {
        guard row >= 0, let bits = highlights.value(for: HighlightKey(row: row, column: column)) else { return false }
        let local = row % HighlightKey.rowCount
        return local / 8 < bits.count && bits[bits.startIndex + local / 8] & (1 << (local % 8)) != 0
    }

    /// Group the demand's missing blocks into runs of at most `blocksPerRead`.
    private func plannedHighlightReads() -> [HighlightRead] {
        guard let demand = highlightDemand, !demand.columns.isEmpty else { return [] }
        let first = demand.rows.lowerBound / HighlightKey.rowCount
        let last = (demand.rows.upperBound - 1) / HighlightKey.rowCount
        return stride(from: first, through: last, by: Self.blocksPerRead).compactMap { start in
            let blocks = start...min(start + Self.blocksPerRead - 1, last)
            let columns = demand.columns.filter { column in
                blocks.contains { block in
                    let key = HighlightKey(block: block, column: column)
                    return !highlights.contains(key) && !failedHighlights.contains(key)
                }
            }
            return columns.isEmpty ? nil : HighlightRead(blocks: blocks, columns: columns)
        }
    }

    private func loadHighlights() {
        guard highlightTask == nil, let session else { return }
        let reads = plannedHighlightReads()
        guard !reads.isEmpty else { return }
        let findRequest = request
        let worker = worker
        let gate = gate
        highlightTask = Task { [weak self] in
            do {
                let results: [FindHighlight] = try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        continuation.resume(
                            with: Result {
                                try reads.map { read in
                                    guard gate.isCurrent(findRequest) else { throw CancellationError() }
                                    return try session.highlight(
                                        columns: read.columns,
                                        start: UInt64(read.blocks.lowerBound * HighlightKey.rowCount),
                                        count: UInt32(read.blocks.count * HighlightKey.rowCount))
                                }
                            })
                    }
                }
                guard !Task.isCancelled, let self, self.request == findRequest else { return }
                self.highlightTask = nil
                for (read, result) in zip(reads, results) { self.store(result, of: read) }
                self.highlightRevision &+= 1
                self.loadHighlights()
            } catch {
                guard !Task.isCancelled, let self, self.request == findRequest else { return }
                self.highlightTask = nil
                if error is CancellationError { return }
                for read in reads {
                    for block in read.blocks {
                        for column in read.columns {
                            self.failedHighlights.insert(HighlightKey(block: block, column: column))
                        }
                    }
                }
                self.onError?(error, "Find highlight error: ")
                self.loadHighlights()
            }
        }
    }

    /// Split each column's bitmap into its blocks. Blocks past the last row are
    /// stored empty, so they are not requested again.
    private func store(_ result: FindHighlight, of read: HighlightRead) {
        let bytesPerBlock = HighlightKey.rowCount / 8
        for column in result.columns {
            for (index, block) in read.blocks.enumerated() {
                let start = min(index * bytesPerBlock, column.matched.count)
                let end = min(start + bytesPerBlock, column.matched.count)
                let bits = Data(column.matched[start..<end])
                highlights.insert(bits, for: HighlightKey(block: block, column: column.column), cost: bits.count + 16)
            }
        }
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
                                session: session, cursor: cursor, backwards: backwards, limit: limit)
                            // A boundary wraps only after an actual cursor. The first
                            // empty scan establishes zero matches without scanning twice.
                            if page.isEmpty, cursor != nil {
                                page = try readFindPage(
                                    session: session, cursor: nil, backwards: backwards, limit: limit)
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
                if error is CancellationError { return }
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
    session: FindSession, cursor: CellMatch?, backwards: Bool, limit: UInt32
) throws -> [CellMatch] {
    backwards
        ? try session.previous(before: cursor, limit: limit)
        : try session.next(after: cursor, limit: limit)
}
