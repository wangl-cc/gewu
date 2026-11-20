import AppKit
import Foundation
import GewuKit
import Observation
import Synchronization
import Testing
import os

/// Drives the native table the way SwiftUI does: every model change is
/// followed by `present(_:)` with a fresh ``TablePresentation``.
@Suite @MainActor
struct TablePresentationTests {
    private static let csv = "number,label\n1,first\n2,second\n3,third\n"
    private static let style = TablePresentation.Style(
        fontSize: 12, rowHeight: 28, showsRowNumbers: true, rowNumberBase: 0)

    /// A table whose reads for rows other than `immediate` wait for the test.
    @MainActor
    private final class Harness {
        let model: TableModel
        let controller: TableViewController
        let table: NSTableView
        private let reads = ReadQueue()
        var style = TablePresentationTests.style

        init(_ csv: String, gated: Bool, blockByteLimit: Int = 1024 * 1024) async throws {
            let model = try await TestTables.model(csv)
            self.model = model
            let immediate = model.displayed
            let reads = reads
            let texts = TableTextCache(blockByteLimit: blockByteLimit) { rows, key in
                if !gated || rows === immediate { return try await rows.text(of: key.column, rows: key.rows) }
                return try await withCheckedThrowingContinuation { pending in
                    reads.push(GatedRead(rows: rows, key: key, continuation: pending))
                }
            }
            controller = TableViewController(
                model: model, presentation: TablePresentation(model: model, inspectorVisible: false, style: style),
                texts: texts)
            controller.loadViewIfNeeded()
            controller.view.frame = NSRect(x: 0, y: 0, width: 640, height: 240)
            controller.view.layoutSubtreeIfNeeded()
            let scroll = try #require(controller.view.subviews.compactMap { $0 as? NSScrollView }.first)
            table = try #require(scroll.documentView as? NSTableView)
            table.layoutSubtreeIfNeeded()
            present()
        }

        func present() {
            controller.present(TablePresentation(model: model, inspectorVisible: false, style: style))
        }

        func layout() {
            controller.view.layoutSubtreeIfNeeded()
            table.layoutSubtreeIfNeeded()
            present()
        }

        func cell(_ column: ColumnID, _ row: Int) throws -> MatchCellView {
            let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column.raw)"))
            #expect(index >= 0)
            return try #require(table.view(atColumn: index, row: row, makeIfNecessary: true) as? MatchCellView)
        }

        func loaded(_ column: ColumnID, _ row: Int) async throws -> String? {
            let cell = try cell(column, row)
            await eventually { !cell.isLoading }
            return cell.isLoading ? nil : cell.textField?.stringValue
        }

        /// Wait for a query to finish and present its rows.
        func settle() async {
            await eventually { !self.model.isUpdating }
            present()
        }

        /// The next gated read, waiting a moment for one to arrive.
        func nextRead(within timeout: Duration = .seconds(5)) async -> GatedRead? {
            await eventually({ !self.reads.isEmpty }, timeout: timeout)
            return reads.pop()
        }

        /// Finish reads until none arrive for a moment.
        func finishAll() async {
            while let read = await nextRead(within: .milliseconds(200)) { await read.finish() }
        }

        func stop() {
            controller.stop()
            model.stop()
            while let read = reads.pop() { read.continuation.resume(throwing: CancellationError()) }
        }
    }

    @Test func newRowsWaitUntilEveryVisibleValueIsReady() async throws {
        let longLabel = "second" + String(repeating: "x", count: 1024)
        let harness = try await Harness(
            "number,label\n1,first\n2,\(longLabel)\n3,third\n", gated: true, blockByteLimit: 256)
        defer { harness.stop() }
        let (number, label) = (harness.model.id("number"), harness.model.id("label"))
        #expect(try await harness.loaded(number, 0) == "1")
        let first = try harness.cell(number, 0)

        harness.model.setFilter(
            Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: number)
        await harness.settle()
        let pending = try #require(await harness.nextRead())
        #expect(harness.table.numberOfRows == 3 && first.textField?.stringValue == "1" && !first.isLoading)
        await pending.finish()
        await harness.finishAll()
        await eventually { harness.table.numberOfRows == 2 }

        // No wait for cells after the switch: every value, including ones larger
        // than the cache budget, was installed with the rows.
        #expect(harness.table.numberOfRows == 2)
        let numberCell = try harness.cell(number, 0)
        let labelCell = try harness.cell(label, 0)
        #expect(!numberCell.isLoading && numberCell.textField?.stringValue == "2")
        #expect(!labelCell.isLoading && labelCell.textField?.stringValue == longLabel)
        #expect(harness.controller.rowNumberLabels(0..<1) == ["1"])
        harness.table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)

        // A later style update keeps the rows, selection and uncached large values.
        harness.style = .init(fontSize: 16, rowHeight: 32, showsRowNumbers: false, rowNumberBase: 1)
        harness.present()
        #expect(harness.table.numberOfRows == 2 && harness.table.selectedRow == 0)
        #expect(!labelCell.isLoading && labelCell.textField?.stringValue == longLabel)
    }

    @Test func aSupersededSwitchNeverReplacesTheNewerRows() async throws {
        let harness = try await Harness(Self.csv, gated: true)
        defer { harness.stop() }
        let number = harness.model.id("number")
        #expect(try await harness.loaded(number, 0) == "1")

        harness.model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: number)
        await harness.settle()
        let older = try #require(await harness.nextRead())
        harness.model.setFilter(Filter(condition: .range(min: "3", max: nil), includeNulls: false), column: number)
        await harness.settle()
        let newer = try #require(await harness.nextRead())
        #expect(newer.rows === harness.model.displayed)
        await newer.finish()
        await harness.finishAll()
        await eventually { harness.table.numberOfRows == 1 }
        await older.finish()
        for _ in 0..<20 { await Task.yield() }
        #expect(harness.table.numberOfRows == 1)
        #expect(try harness.cell(number, 0).textField?.stringValue == "3")
    }

    @Test func scrollingDuringPreparationReadsTheNewViewport() async throws {
        let harness = try await Harness(
            "number,label\n" + (0..<300).map { "\($0),row\($0)" }.joined(separator: "\n"), gated: true)
        defer { harness.stop() }
        let number = harness.model.id("number")
        #expect(try await harness.loaded(number, 0) == "0")

        harness.model.setSortDirection(column: number, descending: true)
        await harness.settle()
        let firstRead = try #require(await harness.nextRead())
        #expect(firstRead.key.start == 0)
        harness.table.scrollRowToVisible(180)
        harness.layout()
        await firstRead.finish()
        let moved = try #require(await harness.nextRead())
        #expect(moved.key.start >= 128, "The next read follows the scrolled viewport")
        await moved.finish()
        await harness.finishAll()
        let visible = harness.table.rows(in: harness.table.visibleRect)
        await eventually {
            (try? harness.cell(number, visible.location))?.textField?.stringValue == String(299 - visible.location)
        }
        #expect(visible.location > 100)
        for row in visible.location..<(visible.location + visible.length) {
            let cell = try harness.cell(number, row)
            #expect(!cell.isLoading && cell.textField?.stringValue == String(299 - row))
        }
    }

    @Test func presentationReadsOnlyStateTheTableShows() async throws {
        let model = try await TestTables.model(Self.csv)
        defer { model.stop() }
        let (number, label) = (model.id("number"), model.id("label"))
        let updates = OSAllocatedUnfairLock(initialState: 0)
        func track() {
            withObservationTracking {
                _ = TablePresentation(model: model, inspectorVisible: true, style: Self.style)
            } onChange: {
                updates.withLock { $0 += 1 }
            }
        }
        track()
        model.chartX = number
        model.showsSorting = true
        model.setSortDirection(column: number, descending: true)
        #expect(model.isUpdating)
        #expect(updates.withLock { $0 } == 0, "Pending queries and unrelated state are not table inputs")
        model.clearSorts()
        #expect(!model.isUpdating && updates.withLock { $0 } == 0)

        model.select(label)
        #expect(updates.withLock { $0 } == 1)
        track()
        model.select(label)
        #expect(updates.withLock { $0 } == 1, "Selecting the selected column does not republish")
    }

    @Test func showingAndHidingColumnsKeepsRowsAndLoadedText() async throws {
        let harness = try await Harness("a,b,c\n1,first,10\n2,second,20\n3,third,30\n", gated: false)
        defer { harness.stop() }
        let (a, b) = (harness.model.id("a"), harness.model.id("b"))
        let rows = harness.model.displayed
        #expect(try await harness.loaded(a, 0) == "1")
        #expect(try await harness.loaded(b, 0) == "first")

        harness.model.setColumnsVisible([b], visible: false)
        harness.present()
        #expect(!harness.model.isUpdating && harness.model.displayed === rows)
        let remaining = try harness.cell(a, 0)
        #expect(!remaining.isLoading && remaining.textField?.stringValue == "1")

        harness.model.setColumnsVisible([b], visible: true)
        harness.present()
        #expect(harness.model.displayed === rows)
        let restored = try harness.cell(b, 0)
        #expect(!restored.isLoading && restored.textField?.stringValue == "first", "Restored from the cache")
    }

    @Test func cellsSurviveFindDecorationAndSameSizeSorting() async throws {
        let harness = try await Harness(Self.csv, gated: false)
        defer { harness.stop() }
        let (number, label) = (harness.model.id("number"), harness.model.id("label"))
        #expect(try await harness.loaded(number, 0) == "1")
        #expect(try await harness.loaded(label, 0) == "first")
        let labelCell = try harness.cell(label, 0)

        harness.model.find.query = "first"
        await eventually { harness.model.find.hasMatches == true }
        harness.model.find.requestHighlights(rows: 0..<3, columns: [number, label])
        await eventually { !harness.model.find.highlights.isEmpty }
        harness.present()
        #expect(try harness.cell(label, 0) === labelCell)
        #expect(labelCell.currentMatch)
        #expect(
            labelCell.textField?.attributedStringValue.attribute(.backgroundColor, at: 0, effectiveRange: nil)
                as? NSColor == .findHighlightColor)

        harness.model.find.query = ""
        harness.present()
        #expect(try harness.cell(label, 0) === labelCell && !labelCell.currentMatch)

        harness.model.setSortDirection(column: number, descending: true)
        await harness.settle()
        await eventually { (try? harness.cell(number, 0))?.textField?.stringValue == "3" }
        #expect(try harness.cell(label, 0) === labelCell && labelCell.textField?.stringValue == "third")
        #expect(harness.table.numberOfRows == 3)
    }

    @Test func columnReorderKeepsSelectionAndLoadedValues() async throws {
        let harness = try await Harness(Self.csv, gated: false)
        defer { harness.stop() }
        let (number, label) = (harness.model.id("number"), harness.model.id("label"))
        let rows = harness.model.displayed
        #expect(try await harness.loaded(number, 1) == "2")
        #expect(try await harness.loaded(label, 1) == "second")
        harness.table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)

        harness.model.reorderColumns([label, number])
        harness.present()
        #expect(harness.model.displayed === rows)
        #expect(harness.table.selectedRowIndexes == IndexSet(integer: 1))
        #expect(harness.controller.displayedColumns == [label, number])
        // Checked synchronously: a reset-and-refetch implementation cannot pass.
        #expect(try harness.cell(label, 1).textField?.stringValue == "second")
        #expect(try harness.cell(number, 1).textField?.stringValue == "2")
    }

    @Test func nativeColumnDragUpdatesTheModelAndKeepsTheGutterFixed() async throws {
        let harness = try await Harness(Self.csv, gated: false)
        defer { harness.stop() }
        let (number, label) = (harness.model.id("number"), harness.model.id("label"))
        let controller = harness.controller
        #expect(harness.table.allowsColumnReordering)
        #expect(controller.tableView(harness.table, shouldReorderColumn: 2, toColumn: -1))
        #expect(controller.tableView(harness.table, shouldReorderColumn: 2, toColumn: 1))
        #expect(!controller.tableView(harness.table, shouldReorderColumn: 2, toColumn: 0))
        #expect(!controller.tableView(harness.table, shouldReorderColumn: 0, toColumn: -1))

        // A drag during a failing query keeps the new order: columns are not part of queries.
        harness.model.setFilter(
            Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: number)
        harness.table.moveColumn(2, toColumn: 1)
        #expect(harness.model.visibleColumns == [label, number])
        await harness.settle()
        #expect(harness.model.visibleColumns == [label, number])
        #expect(controller.displayedColumns == [label, number])
    }

    @Test func rowNumbersAreAvailableImmediatelyAfterScrollingBack() async throws {
        let harness = try await Harness("value\n" + (0..<300).map(String.init).joined(separator: "\n"), gated: false)
        defer { harness.stop() }
        harness.model.setSortDirection(column: harness.model.id("value"), descending: true)
        await harness.settle()
        await eventually { harness.controller.shownRows === harness.model.displayed }
        harness.table.scrollRowToVisible(250)
        harness.layout()
        harness.table.scrollRowToVisible(0)
        harness.layout()
        // No wait for any read: the gutter maps rows through the shown rows.
        #expect(harness.controller.rowNumberLabels(0..<3) == ["299", "298", "297"])
        #expect(harness.controller.rowNumberLabels(298..<400) == ["1", "0"], "Clipped to the available rows")
    }

    @Test func changingRowCountsRebindValuesThroughEmptyResults() async throws {
        let harness = try await Harness(Self.csv, gated: false)
        defer { harness.stop() }
        let (number, label) = (harness.model.id("number"), harness.model.id("label"))
        #expect(try await harness.loaded(number, 0) == "1")

        func filter(_ minimum: String) async {
            harness.model.setFilter(
                Filter(condition: .range(min: minimum, max: nil), includeNulls: false), column: number)
            await harness.settle()
            await eventually { harness.controller.shownRows === harness.model.displayed }
            harness.layout()
        }
        await filter("2")
        #expect(harness.table.numberOfRows == 2)
        #expect(try await harness.loaded(number, 0) == "2")
        #expect(try await harness.loaded(number, 1) == "3")
        #expect(try await harness.loaded(label, 0) == "second")
        #expect(harness.controller.rowNumberLabels(0..<2) == ["1", "2"])
        harness.table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)

        await filter("100")
        #expect(harness.table.numberOfRows == 0 && harness.table.selectedRow == -1)
        #expect(harness.controller.rowNumberLabels(0..<1).isEmpty)

        harness.model.clearFilters()
        await harness.settle()
        await eventually { harness.controller.shownRows === harness.model.displayed }
        harness.layout()
        #expect(harness.table.numberOfRows == 3)
        #expect(try await harness.loaded(number, 0) == "1")
        #expect(try await harness.loaded(label, 0) == "first")
        #expect(harness.controller.rowNumberLabels(0..<3) == ["0", "1", "2"])
    }

    @Test func hidingColumnsDuringPreparationReadsTheColumnsThatBecomeVisible() async throws {
        let header = (0..<16).map { "c\($0)" }.joined(separator: ",")
        let rows = (0..<3).map { row in (0..<16).map { String(row * 100 + $0) }.joined(separator: ",") }
        let harness = try await Harness(header + "\n" + rows.joined(separator: "\n"), gated: true)
        defer { harness.stop() }
        let columns = harness.model.source.columns.map(\.id)
        #expect(try await harness.loaded(columns[0], 0) == "0")

        harness.model.setSortDirection(column: columns[0], descending: true)
        await harness.settle()
        let firstRead = try #require(await harness.nextRead())
        // Hiding columns brings c15 into the viewport before the switch.
        harness.model.setColumnsVisible(Array(columns[1..<14]), visible: false)
        harness.present()
        await firstRead.finish()
        var keys: [BlockKey] = [firstRead.key]
        while let next = await harness.nextRead(within: .milliseconds(200)) {
            keys.append(next.key)
            await next.finish()
        }
        #expect(keys.contains { $0.column == columns[15] })
        await eventually { harness.controller.shownRows === harness.model.displayed }
        #expect(try harness.cell(columns[0], 0).textField?.stringValue == "200")
        #expect(try harness.cell(columns[15], 0).textField?.stringValue == "215")
    }
}

/// Reads waiting for the test, filled from the cache's read worker.
private final class ReadQueue: Sendable {
    private let reads = Mutex<[GatedRead]>([])

    var isEmpty: Bool { reads.withLock { $0.isEmpty } }
    func push(_ read: GatedRead) { reads.withLock { $0.append(read) } }
    func pop() -> GatedRead? { reads.withLock { $0.isEmpty ? nil : $0.removeFirst() } }
}

private struct GatedRead: Sendable {
    let rows: DisplayRows
    let key: BlockKey
    let continuation: CheckedContinuation<ColumnText, any Error>

    func finish() async {
        do {
            continuation.resume(returning: try await rows.text(of: key.column, rows: key.rows))
        } catch {
            continuation.resume(throwing: error)
        }
    }
}
