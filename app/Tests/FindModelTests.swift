import Foundation
import GewuKit
import Observation
import Testing
import os

@Suite @MainActor
struct FindModelTests {
    @Test func emptyInvalidationDoesNotPublishDisplayChanges() {
        let find = FindModel(alerts: Alerts())
        let publications = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            _ = (find.query, find.currentMatch, find.isFinding, find.hasMatches, find.highlights, find.status)
        } onChange: {
            publications.withLock { $0 += 1 }
        }
        find.invalidate()
        find.stop()
        #expect(publications.withLock { $0 } == 0)
    }

    @Test func replacedQueriesNavigateAndHighlightTheLatestSearch() async throws {
        let model = try await TestTables.model("value,number\nold,120\nnew,12\nnew,12\n")
        defer { model.stop() }
        let value = model.id("value")
        let number = model.id("number")
        let rows = model.displayed

        // A debounced query replaced before its first page runs never navigates.
        model.find.query = "old"
        model.find.query = "12"
        await eventually { model.find.hasMatches != nil }
        #expect(model.find.currentMatch == CellMatch(displayRow: 1, column: number))
        #expect(model.displayed === rows, "Find never changes the rows")
        #expect(model.reveal?.row == 1, "Navigation asks the table to reveal the match")

        // Only the requested on-screen column is read and kept.
        model.find.requestHighlights(rows: 0..<3, columns: [number])
        await eventually { !model.find.highlights.isEmpty }
        #expect((0..<3).map { model.find.isMatch(row: $0, column: number) } == [false, true, true])
        #expect(!(0..<3).contains { model.find.isMatch(row: $0, column: value) })
        let blocks = model.find.highlights
        model.find.requestHighlights(rows: 1..<3, columns: [number])
        #expect(model.find.highlights == blocks, "Loaded blocks are not requested again")

        model.find.next()
        await eventually { model.find.currentMatch?.displayRow == 2 }
        model.find.next()
        await eventually { model.find.currentMatch?.displayRow == 1 }
        model.find.next(backwards: true)
        await eventually { model.find.currentMatch?.displayRow == 2 }

        model.find.query = "old"
        await eventually { model.find.currentMatch?.column == value }
        model.find.query = ""
        #expect(model.find.currentMatch == nil && model.find.highlights.isEmpty && !model.find.canNavigate)
    }

    @Test func highlightsCoverTheVisibleBlocksIncludingThePartialLastBlock() async throws {
        let model = try await TestTables.model("left,right\n" + String(repeating: "needle,needle\n", count: 263))
        defer { model.stop() }
        let left = model.id("left")
        let right = model.id("right")
        model.find.query = "needle"
        await eventually { model.find.hasMatches == true }
        model.find.requestHighlights(rows: 0..<64, columns: [left])
        await eventually { model.find.isMatch(row: 0, column: left) }

        let rows = 63..<263
        model.find.requestHighlights(rows: rows, columns: [left, right])
        await eventually { model.find.isMatch(row: 262, column: right) }
        #expect(rows.allSatisfy { model.find.isMatch(row: $0, column: left) })
        #expect(rows.allSatisfy { model.find.isMatch(row: $0, column: right) })
        #expect(!model.find.isMatch(row: rows.upperBound, column: right))
    }

    @Test func aRunningQueryPausesFindUntilItsRowsShow() async throws {
        let model = try await TestTables.model("value,number\nold,120\nnew,12\nnew,12\n")
        defer { model.stop() }
        let number = model.id("number")
        model.find.query = "12"
        await eventually { model.find.hasMatches == true }

        model.setSortDirection(column: number, descending: true)
        #expect(model.isUpdating && !model.find.canNavigate, "Matches of the old rows are dropped")
        model.find.query = "120"
        await eventually { !model.isUpdating && model.find.hasMatches == true }
        #expect(model.find.currentMatch == CellMatch(displayRow: 0, column: number))
    }

    @Test func highlightsFollowScrollingWithoutDroppingReadsAndCoverTallViewports() async throws {
        let values = (0..<400).map { $0 % 2 == 0 ? "hit" : "miss" }
        let model = try await TestTables.model("value\n" + values.joined(separator: "\n"))
        defer { model.stop() }
        let value = model.id("value")
        model.find.query = "hit"
        await eventually { model.find.hasMatches == true }

        // Scrolling before the first read completes must not drop that read.
        model.find.requestHighlights(rows: 0..<10, columns: [value])
        model.find.requestHighlights(rows: 64..<74, columns: [value])
        await eventually { model.find.isMatch(row: 0, column: value) && model.find.isMatch(row: 64, column: value) }
        // A viewport taller than one Core highlight block is read in one request.
        model.find.requestHighlights(rows: 0..<400, columns: [value])
        await eventually { model.find.isMatch(row: 398, column: value) }
        #expect((0..<400).allSatisfy { model.find.isMatch(row: $0, column: value) == ($0 % 2 == 0) })
    }

    @Test func burstNavigationTakesEveryStep() async throws {
        let model = try await TestTables.model("value\n" + (0..<40).map { _ in "new" }.joined(separator: "\n") + "\n")
        defer { model.stop() }
        model.find.query = "new"
        await eventually { model.find.currentMatch?.displayRow == 0 }

        // Presses during a step are taken by the next one, never dropped.
        for _ in 0..<35 { model.find.next() }
        for _ in 0..<3 { model.find.next(backwards: true) }
        await eventually { !model.find.isFinding && model.find.currentMatch?.displayRow == 32 }
        for _ in 0..<10 { model.find.next() }
        await eventually { !model.find.isFinding && model.find.currentMatch?.displayRow == 2 }
        #expect(model.find.currentMatch?.displayRow == 2, "Navigation wraps past the last match")
    }

    @Test func columnReorderPreservesTheCurrentMatch() async throws {
        let alerts = Alerts()
        let model = try await TestTables.model("value,number\nold,120\nnew,12\nnew,12\n", alerts: alerts)
        defer { model.stop() }
        let value = model.id("value")
        let number = model.id("number")
        model.find.query = "12"
        await eventually { model.find.currentMatch?.displayRow == 1 }
        model.find.next()
        await eventually { model.find.currentMatch?.displayRow == 2 }

        model.reorderColumns([number, value])
        #expect(model.find.currentMatch == CellMatch(displayRow: 2, column: number))
        #expect(!model.find.isFinding && alerts.current == nil)
    }

    @Test func invalidQueriesAreReported() async throws {
        let alerts = Alerts()
        let model = try await TestTables.model("value\nx\n", alerts: alerts)
        defer { model.stop() }
        model.find.query = " "
        #expect(model.find.cellFind == nil && alerts.current == nil, "Blank queries start no Find")
    }
}
