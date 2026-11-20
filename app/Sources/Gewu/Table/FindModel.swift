import Foundation
import GewuKit
import Observation

/// Cell Find over the displayed rows: bounded navigation and highlights for
/// the blocks on screen. Find never filters rows or changes analyses.
///
/// Only the current match, one page of matches and a bounded set of
/// highlight blocks live in Swift. Every piece of work is a single task that
/// is cancelled when its search is replaced.
@MainActor
@Observable
final class FindModel {
    enum Status: Equatable {
        case finding
        case noMatches
        /// Press Return to find the first match.
        case ready
        case match(displayRow: Int)
    }

    /// The text in the search field. Editing it restarts Find after a short pause.
    var query = "" {
        didSet {
            if query != oldValue { restart(debounce: true) }
        }
    }
    private(set) var currentMatch: CellMatch?
    private(set) var isFinding = false
    private(set) var hasMatches: Bool?
    /// Match flags of the blocks requested for display.
    private(set) var highlights = HighlightBlocks()
    /// The search behind the current state; `nil` without a query or while rows are updating.
    private(set) var search: CellSearch?

    @ObservationIgnored var onNavigate: ((CellMatch) -> Void)?
    @ObservationIgnored private let alerts: Alerts
    @ObservationIgnored private var rows: DisplayRows?
    @ObservationIgnored private var columns: [ColumnID] = []
    @ObservationIgnored private var navigation: Task<Void, Never>?
    @ObservationIgnored private var highlighting: Task<Void, Never>?
    /// The latest on-screen demand. A read in flight is never cancelled for a
    /// newer demand: its blocks stay useful, and the next read follows the latest one.
    @ObservationIgnored private var demand: (rows: Range<Int>, columns: [ColumnID])?
    /// Blocks whose read failed for this search; they are not retried.
    @ObservationIgnored private var failedBlocks: Set<HighlightBlocks.Key> = []
    /// The last page, ascending in display order, and the current match's
    /// index in it. Steps inside the page are immediate, so quick presses are never lost.
    @ObservationIgnored private var page: [CellMatch] = []
    @ObservationIgnored private var pageIndex: Int?
    /// Net steps the user asked for; positive is forward.
    @ObservationIgnored private var pendingSteps = 0

    private static let pageSize = 32

    init(alerts: Alerts) {
        self.alerts = alerts
    }

    var activeQuery: String? {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return search == nil || trimmed.isEmpty ? nil : trimmed
    }

    var canNavigate: Bool { search != nil && hasMatches != false }

    var status: Status? {
        guard search != nil else { return nil }
        if hasMatches == false { return .noMatches }
        if let currentMatch { return .match(displayRow: currentMatch.displayRow) }
        return isFinding ? .finding : .ready
    }

    func isMatch(row: Int, column: ColumnID) -> Bool { highlights.isMatch(row: row, column: column) }

    /// Search `rows` in `columns`. With `keepingMatch`, the rows and query are
    /// unchanged and only the column order moved, so the current match stays.
    func update(rows: DisplayRows, columns: [ColumnID], keepingMatch: Bool = false) {
        let match = keepingMatch && rows === self.rows ? currentMatch : nil
        self.rows = rows
        self.columns = columns
        restart(debounce: false, keeping: match)
    }

    /// Clear results until the next ``update(rows:columns:keepingMatch:)``.
    func invalidate() {
        rows = nil
        reset()
    }

    func stop() { invalidate() }

    func next(backwards: Bool = false) {
        guard let search else { return }
        pendingSteps += backwards ? -1 : 1
        pump(search, debounce: false)
    }

    /// Fetch flags for the on-screen rows of the given columns, reading only blocks not loaded yet.
    func requestHighlights(rows: Range<Int>, columns: [ColumnID]) {
        guard search != nil, !rows.isEmpty, rows.lowerBound >= 0 else { return }
        let scope = Set(self.columns)
        demand = (rows, columns.filter(scope.contains))
        loadHighlights()
    }

    // MARK: Search lifecycle

    private func reset() {
        navigation?.cancel()
        highlighting?.cancel()
        navigation = nil
        highlighting = nil
        if search != nil { search = nil }
        if currentMatch != nil { currentMatch = nil }
        if isFinding { isFinding = false }
        if hasMatches != nil { hasMatches = nil }
        if !highlights.isEmpty { highlights = HighlightBlocks() }
        page = []
        pageIndex = nil
        pendingSteps = 0
        demand = nil
        failedBlocks = []
    }

    private func restart(debounce: Bool, keeping match: CellMatch? = nil) {
        reset()
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let rows, !trimmed.isEmpty else { return }
        let search: CellSearch
        do {
            search = try rows.search(trimmed, in: columns)
        } catch {
            alerts.report(error, title: "Could Not Find")
            return
        }
        self.search = search
        if let match {
            currentMatch = match
            hasMatches = true
        } else {
            isFinding = true
            pendingSteps = 1
            pump(search, debounce: debounce)
        }
    }

    // MARK: Navigation

    /// Consume queued steps from the cached page, or start one fetch when the
    /// page is exhausted. Each iteration moves one step or starts exactly one
    /// fetch, so a burst of presses is never coalesced away.
    private func pump(_ search: CellSearch, debounce: Bool) {
        while pendingSteps != 0, navigation == nil {
            let backwards = pendingSteps < 0
            guard let index = pageIndex, page.indices.contains(index + (backwards ? -1 : 1)) else {
                fetch(search, backwards: backwards, debounce: debounce)
                return
            }
            pageIndex = index + (backwards ? -1 : 1)
            pendingSteps += backwards ? 1 : -1
            publish(page[index + (backwards ? -1 : 1)])
        }
    }

    /// Fetch the page beyond the current match, wrapping around at either end.
    private func fetch(_ search: CellSearch, backwards: Bool, debounce: Bool) {
        isFinding = true
        let cursor = currentMatch
        let limit = Self.pageSize
        navigation = Task {
            do {
                if debounce { try await Task.sleep(for: .milliseconds(150)) }
                var page = try await Self.page(of: search, from: cursor, backwards: backwards, limit: limit)
                // Wrap only after an actual cursor; the first empty scan means no matches at all.
                if page.isEmpty, cursor != nil {
                    page = try await Self.page(of: search, from: nil, backwards: backwards, limit: limit)
                }
                try Task.checkCancellation()
                navigation = nil
                apply(page, backwards: backwards)
                pump(search, debounce: false)
            } catch is CancellationError {
            } catch {
                navigation = nil
                isFinding = false
                alerts.report(error, title: "Could Not Find")
            }
        }
    }

    private static func page(
        of search: CellSearch, from cursor: CellMatch?, backwards: Bool, limit: Int
    ) async throws -> [CellMatch] {
        backwards
            ? try await search.matches(before: cursor, limit: limit)
            : try await search.matches(after: cursor, limit: limit)
    }

    private func apply(_ newPage: [CellMatch], backwards: Bool) {
        guard pendingSteps != 0 else {
            // Forward and backward presses cancelled out while the page loaded.
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

    // MARK: Highlights

    /// One read: consecutive blocks and every column missing in any of them.
    private struct HighlightRead {
        let blocks: ClosedRange<Int>
        let columns: [ColumnID]
    }

    /// Group the demand's missing blocks into runs Core can answer in one call,
    /// so each column's matcher compiles once per run.
    private func plannedReads() -> [HighlightRead] {
        guard let demand, !demand.columns.isEmpty else { return [] }
        let size = HighlightBlocks.rowsPerBlock
        let perRead = max(1, CellSearch.maximumHighlightRows / size)
        let first = demand.rows.lowerBound / size
        let last = (demand.rows.upperBound - 1) / size
        return stride(from: first, through: last, by: perRead).compactMap { start in
            let blocks = start...min(start + perRead - 1, last)
            let columns = demand.columns.filter { column in
                blocks.contains { block in
                    let key = HighlightBlocks.Key(block: block, column: column)
                    return !highlights.contains(key) && !failedBlocks.contains(key)
                }
            }
            return columns.isEmpty ? nil : HighlightRead(blocks: blocks, columns: columns)
        }
    }

    private func loadHighlights() {
        guard highlighting == nil, let search else { return }
        let reads = plannedReads()
        guard !reads.isEmpty else { return }
        let size = HighlightBlocks.rowsPerBlock
        highlighting = Task {
            do {
                for read in reads {
                    let rows = (read.blocks.lowerBound * size)..<((read.blocks.upperBound + 1) * size)
                    let flags = try await search.highlights(read.columns, rows: rows)
                    try Task.checkCancellation()
                    highlights.store(flags, blocks: read.blocks, columns: read.columns)
                }
                highlighting = nil
                loadHighlights()
            } catch is CancellationError {
            } catch {
                highlighting = nil
                for read in reads {
                    for block in read.blocks {
                        for column in read.columns { failedBlocks.insert(.init(block: block, column: column)) }
                    }
                }
                alerts.report(error, title: "Could Not Highlight Matches")
                loadHighlights()
            }
        }
    }
}

/// Match flags of displayed blocks, bounded by count. A value, so storing
/// new blocks notifies the table to redraw.
struct HighlightBlocks: Equatable {
    static let rowsPerBlock = 64
    private static let capacity = 2048

    struct Key: Hashable {
        let block: Int
        let column: ColumnID
    }

    /// One bit per row of the block, low bit first.
    private var blocks: [Key: [UInt8]] = [:]
    /// Keys in insertion order, oldest first.
    private var order: [Key] = []

    var isEmpty: Bool { blocks.isEmpty }

    func contains(_ key: Key) -> Bool { blocks[key] != nil }

    func isMatch(row: Int, column: ColumnID) -> Bool {
        guard row >= 0, let bits = blocks[Key(block: row / Self.rowsPerBlock, column: column)] else { return false }
        let local = row % Self.rowsPerBlock
        return local / 8 < bits.count && bits[local / 8] & (1 << (local % 8)) != 0
    }

    /// Split a result into blocks. Blocks past the last row are stored empty so
    /// they are not requested again.
    mutating func store(_ flags: SearchHighlights, blocks range: ClosedRange<Int>, columns: [ColumnID]) {
        let bytesPerBlock = Self.rowsPerBlock / 8
        for column in columns {
            let bits = flags.matches[column] ?? []
            for (index, block) in range.enumerated() {
                let start = min(index * bytesPerBlock, bits.count)
                let end = min(start + bytesPerBlock, bits.count)
                let key = Key(block: block, column: column)
                if blocks.updateValue(Array(bits[start..<end]), forKey: key) == nil { order.append(key) }
            }
        }
        if order.count > Self.capacity {
            let evicted = order.prefix(order.count - Self.capacity)
            for key in evicted { blocks[key] = nil }
            order.removeFirst(evicted.count)
        }
    }
}
