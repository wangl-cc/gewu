import Foundation
import GewuKit
import Observation

/// Cell Find over the displayed rows: wrapping navigation and highlights for
/// the blocks on screen. Find never filters rows or changes analyses.
///
/// Only the current match and a bounded set of highlight blocks live in
/// Swift. Every piece of work is a single task that is cancelled when its
/// search is replaced.
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
    /// Net steps the user asked for and no step has taken yet; positive is
    /// forward. Presses during a step are taken together by the next one, so
    /// quick presses are never lost.
    @ObservationIgnored private var pendingSteps = 0

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
        step(search, debounce: false)
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
            step(search, debounce: debounce)
        }
    }

    // MARK: Navigation

    /// Take every pending step in one scan from the current match, wrapping
    /// around at either end. Steps asked for meanwhile follow when it ends.
    private func step(_ search: CellSearch, debounce: Bool) {
        guard pendingSteps != 0, navigation == nil else { return }
        isFinding = true
        let steps = pendingSteps
        pendingSteps = 0
        let cursor = currentMatch
        navigation = Task {
            do {
                if debounce { try await Task.sleep(for: .milliseconds(150)) }
                let match = try await search.match(steps, from: cursor)
                try Task.checkCancellation()
                navigation = nil
                isFinding = false
                guard let match else {
                    pendingSteps = 0
                    currentMatch = nil
                    hasMatches = false
                    return
                }
                currentMatch = match
                hasMatches = true
                onNavigate?(match)
                step(search, debounce: false)
            } catch is CancellationError {
            } catch {
                navigation = nil
                isFinding = false
                alerts.report(error, title: "Could Not Find")
            }
        }
    }

    // MARK: Highlights

    /// One read: the demand's blocks and every column missing in any of them,
    /// so each column's matcher compiles once per read.
    private struct HighlightRead {
        let blocks: ClosedRange<Int>
        let columns: [ColumnID]
    }

    private func plannedRead() -> HighlightRead? {
        guard let demand, !demand.columns.isEmpty else { return nil }
        let size = HighlightBlocks.rowsPerBlock
        let blocks = (demand.rows.lowerBound / size)...((demand.rows.upperBound - 1) / size)
        let columns = demand.columns.filter { column in
            blocks.contains { block in
                let key = HighlightBlocks.Key(block: block, column: column)
                return !highlights.contains(key) && !failedBlocks.contains(key)
            }
        }
        return columns.isEmpty ? nil : HighlightRead(blocks: blocks, columns: columns)
    }

    private func loadHighlights() {
        guard highlighting == nil, let search, let read = plannedRead() else { return }
        let size = HighlightBlocks.rowsPerBlock
        highlighting = Task {
            do {
                let rows = (read.blocks.lowerBound * size)..<((read.blocks.upperBound + 1) * size)
                let flags = try await search.highlights(read.columns, rows: rows)
                try Task.checkCancellation()
                highlights.store(flags, blocks: read.blocks, columns: read.columns)
                highlighting = nil
                loadHighlights()
            } catch is CancellationError {
            } catch {
                highlighting = nil
                for block in read.blocks {
                    for column in read.columns { failedBlocks.insert(.init(block: block, column: column)) }
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
