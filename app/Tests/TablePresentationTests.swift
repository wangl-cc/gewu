import AppKit
import Foundation
import Observation
import Testing
import os

@Suite @MainActor
struct TablePresentationTests {
    private static let c0 = ColumnId(raw: 0)
    private static let c1 = ColumnId(raw: 1)

    @Test func replacementKeepsOldRowsUntilEveryVisibleValueIsReady() async throws {
        let longLabel = "second" + String(repeating: "x", count: 1024)
        let model = try makeModel(csv: "number,label\n1,first\n2,\(longLabel)\n3,third\n")
        let original = model.ordered
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let loader = gatedLoader(original: original, requests: requests.continuation, batchByteLimit: 256)
        let controller = DataTableController(model: model, loader: loader)
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let first = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !first.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        let pending = try #require(await reads.next())
        controller.update(showsRowNumbers: true)
        #expect(model.isBusy && model.ordered === original)
        #expect(model.appliedFilter(for: Self.c0) == nil)
        #expect(table.numberOfRows == 3 && first.textField?.stringValue == "1" && !first.isLoading)
        pending.finish()
        try await waitUntil { !model.isBusy }

        // No wait for cell delivery after publication: all values, including
        // ones larger than the cache budget, must already be installed.
        #expect(table.numberOfRows == 2)
        let number = try cell(table, column: Self.c0, row: 0)
        let label = try cell(table, column: Self.c1, row: 0)
        #expect(!number.isLoading && number.textField?.stringValue == "2")
        #expect(!label.isLoading && label.textField?.stringValue == longLabel)
        #expect(loader.sourceRow(for: 0) == 1)
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)

        // A later presentation update must keep the committed rows and uncached
        // large values, without clearing the native selection or refetching.
        let committed = model.ordered
        controller.update(showsRowNumbers: false, fontSize: 16, rowHeight: 32, rowNumberBase: 1)
        #expect(model.ordered === committed && table.numberOfRows == 2)
        #expect(table.selectedRow == 0)
        #expect(!label.isLoading && label.textField?.stringValue == longLabel)
    }

    @Test func supersededViewportAndReadFailureCannotReplaceTheDisplayedData() async throws {
        let model = try makeModel()
        let original = model.ordered
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let controller = DataTableController(
            model: model, loader: gatedLoader(original: original, requests: requests.continuation))
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let first = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !first.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        let older = try #require(await reads.next())
        model.setFilter(Filter(condition: .range(min: "3", max: nil), includeNulls: false), column: Self.c0)
        let newer = try #require(await reads.next())
        newer.finish()
        try await waitUntil { !model.isBusy }
        let committed = model.ordered
        #expect(table.numberOfRows == 1 && first.textField?.stringValue == "3" && !first.isLoading)
        older.finish()

        model.clearFilters()
        let failing = try #require(await reads.next())
        failing.continuation.resume(throwing: TablePresentationTimeout())
        try await waitUntil { !model.isBusy }
        #expect(model.ordered === committed && table.numberOfRows == 1)
        #expect(first.textField?.stringValue == "3" && !first.isLoading)
        #expect(model.errorMessage != nil)

        model.clearFilters()
        let closing = try #require(await reads.next())
        model.stop()
        closing.finish()
        for _ in 0..<10 { await Task.yield() }
        #expect(model.ordered === committed && table.numberOfRows == 1)
    }

    @Test func scrollingDuringPreloadLoadsTheNewViewportBeforeCommit() async throws {
        let csv = "number,label\n" + (0..<300).map { "\($0),row\($0)" }.joined(separator: "\n")
        let model = try makeModel(csv: csv)
        let original = model.ordered
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let controller = DataTableController(
            model: model, loader: gatedLoader(original: original, requests: requests.continuation))
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !initial.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setSortDirection(column: Self.c0, descending: true)
        let firstRead = try #require(await reads.next())
        #expect(firstRead.start == 0)
        table.scrollRowToVisible(180)
        controller.view.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)
        firstRead.finish()
        let movedRead = try #require(await reads.next())
        #expect(movedRead.start >= 128)
        #expect(model.ordered === original, "An obsolete viewport cannot commit the new View")
        movedRead.finish()
        try await waitUntil { !model.isBusy }
        let visible = table.rows(in: table.visibleRect)
        #expect(visible.location > 100)
        for row in visible.location..<(visible.location + visible.length) {
            let number = try cell(table, column: Self.c0, row: row)
            #expect(!number.isLoading && number.textField?.stringValue == String(299 - row))
        }
    }

    private func gatedLoader(
        original: GewuOrderedView, requests: AsyncStream<PendingViewportRead>.Continuation,
        batchByteLimit: Int = 1024 * 1024
    ) -> TableSegmentLoader {
        TableSegmentLoader(batchByteLimit: batchByteLimit) { view, keys in
            if view === original { return try TableSegments(reading: view, keys: keys) }
            return try await withCheckedThrowingContinuation { continuation in
                requests.yield(.init(view: view, keys: keys, continuation: continuation))
            }
        }
    }

    @Test func shrinkingRowsAndHidingColumnsCommitTheProjectedViewport() async throws {
        let header = (0..<8).map { "c\($0)" }.joined(separator: ",")
        let data = (0..<300).map { row in (0..<8).map { String(row * 10 + $0) }.joined(separator: ",") }
        let model = try makeModel(csv: header + "\n" + data.joined(separator: "\n"))
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !initial.isLoading }
        table.scrollRowToVisible(180)
        controller.view.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)

        func checkReady() throws {
            let visible = table.rows(in: table.visibleRect)
            #expect(visible.length > 0)
            let indexes = table.columnIndexes(in: table.visibleRect).filter {
                table.tableColumns[$0].identifier.rawValue.hasPrefix("c")
            }
            let columns = indexes.map { ColumnId(raw: UInt32(table.tableColumns[$0].identifier.rawValue.dropFirst())!) }
            let expected = try columns.map {
                try model.ordered.formatColumn(
                    column: $0, start: UInt64(visible.location), count: UInt32(visible.length))
            }
            for (r, row) in (visible.location..<(visible.location + visible.length)).enumerated() {
                for (c, column) in columns.enumerated() {
                    let value = try cell(table, column: column, row: row)
                    #expect(!value.isLoading && value.textField?.stringValue == expected[c].cell(row: r))
                }
            }
        }

        model.setFilter(Filter(condition: .range(min: "2900", max: nil), includeNulls: false), column: Self.c0)
        try await waitUntil { !model.isBusy }
        #expect(table.numberOfRows == 10)
        try checkReady()
        // Removing the filtered column also restores all rows and exposes columns
        // that were outside the old horizontal viewport.
        model.setColumnsVisible([0, 1, 2, 3].map(ColumnId.init), visible: false)
        try await waitUntil { !model.isBusy }
        #expect(table.numberOfRows == 300)
        try checkReady()
    }

    @Test func tableInputPublishesOnlyRelevantWindowChanges() throws {
        let model = try makeModel()
        defer { model.stop() }
        let updates = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            _ = model.tableInput
        } onChange: {
            updates.withLock { $0 += 1 }
        }

        model.inspectorPage = .chart
        model.chartX = Self.c0
        model.chartY = Self.c1
        model.showsSorting = true
        model.setSortDirection(column: Self.c0, descending: true)
        #expect(model.isBusy)
        #expect(updates.withLock { $0 } == 0, "Pending queries and unrelated window state are not table inputs")
        model.clearSort()
        #expect(!model.isBusy && updates.withLock { $0 } == 0)

        model.selectedColumn = Self.c1
        #expect(updates.withLock { $0 } == 1)
        #expect(model.tableInput.selectedColumn == Self.c1)

        withObservationTracking {
            _ = model.tableInput
        } onChange: {
            updates.withLock { $0 += 1 }
        }
        model.selectedColumn = Self.c1
        #expect(updates.withLock { $0 } == 1, "Assigning an unchanged header selection must not republish it")
    }

    @Test func pureColumnVisibilityWithoutConditionsPreservesOrderedViewAndSegmentCache() async throws {
        let model = try makeModel(csv: "a,b,c\n1,first,10\n2,second,20\n3,third,30\n")
        let originalOrdered = model.ordered
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let loader = TableSegmentLoader { view, keys in
            try await withCheckedThrowingContinuation { continuation in
                requests.continuation.yield(.init(view: view, keys: keys, continuation: continuation))
            }
        }
        let controller = DataTableController(model: model, loader: loader)
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        var reads = requests.stream.makeAsyncIterator()
        let initialRead = try #require(await reads.next())
        initialRead.finish()
        let number = try cell(table, column: Self.c0, row: 0)
        let label = try cell(table, column: Self.c1, row: 0)
        try await waitUntil { !number.isLoading && !label.isLoading }
        #expect(loader.segment(for: 0, column: Self.c0).map { $0.segment.texts } == ["1", "2", "3"])

        // Hiding c1 and re-showing c1 synchronously keeps `model.ordered ===` and preserves cached segments.
        model.removeColumn(Self.c1)
        #expect(!model.isBusy && model.ordered === originalOrdered)
        controller.update(showsRowNumbers: true)
        #expect(loader.segment(for: 0, column: Self.c0).map { $0.segment.texts } == ["1", "2", "3"])
        let remaining = try cell(table, column: Self.c0, row: 0)
        #expect(!remaining.isLoading && remaining.textField?.stringValue == "1")

        model.showColumn(Self.c1)
        #expect(!model.isBusy && model.ordered === originalOrdered)
        controller.update(showsRowNumbers: true)
        let restored = try cell(table, column: Self.c1, row: 0)
        #expect(!restored.isLoading && restored.textField?.stringValue == "first")
        #expect(
            loader.segment(for: 0, column: Self.c1).map { $0.segment.texts } == [
                "first", "second", "third",
            ])
    }

    @Test func unchangedAndCancelledBeforeStartQueriesKeepTheAppliedSnapshot() async throws {
        let model = try makeModel()
        defer { model.stop() }
        let original = model.ordered
        model.clearSort()
        model.setFilter(nil, column: Self.c0)
        #expect(!model.isBusy && model.ordered === original)

        // Both edits happen in one main-actor turn, before the query Task can run.
        model.setSortDirection(column: Self.c0, descending: true)
        #expect(model.isBusy)
        model.clearSort()
        #expect(!model.isBusy && model.ordered === original)
        for _ in 0..<10 {
            await Task.yield()
            #expect(model.ordered === original && !model.isBusy)
        }

        // Let cancelled Tasks drain, then use a real query as a worker barrier.
        model.setSortDirection(column: Self.c0, descending: false)
        try await waitUntil { !model.isBusy }
        #expect(model.appliedSort(for: Self.c0)?.descending == false)
        #expect(try model.ordered.formatColumn(column: Self.c0, start: 0, count: 3).texts == ["1", "2", "3"])
        model.clearSort()
        try await waitUntil { !model.isBusy }
        #expect(model.viewer.appliedSorts.isEmpty)
        #expect(try model.ordered.formatColumn(column: Self.c0, start: 0, count: 3).texts == ["1", "2", "3"])
        #expect(model.errorMessage == nil)
    }

    @Test func nativeCellsSurviveFindDecorationAndSameSizeSorting() async throws {
        let model = try makeModel()
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let number = try cell(table, column: Self.c0, row: 0)
        let label = try cell(table, column: Self.c1, row: 0)
        try await waitUntil { !number.isLoading && !label.isLoading }
        #expect(number.textField?.stringValue == "1")

        model.find.query = "first"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }
        model.find.requestHighlight(rows: 0..<3, columns: [Self.c0, Self.c1])
        try await waitUntil { model.find.hasHighlights }
        controller.update(showsRowNumbers: true)
        #expect(try cell(table, column: Self.c1, row: 0) === label)
        #expect(label.currentMatch)
        #expect(
            label.textField?.attributedStringValue.attribute(.backgroundColor, at: 0, effectiveRange: nil) as? NSColor
                == .findHighlightColor)

        model.find.query = ""
        model.updateFind()
        controller.update(showsRowNumbers: true)
        #expect(try cell(table, column: Self.c1, row: 0) === label)
        #expect(!label.currentMatch)

        model.setSortDirection(column: Self.c0, descending: true)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        try await waitUntil { !number.isLoading && number.textField?.stringValue == "3" }
        #expect(try cell(table, column: Self.c0, row: 0) === number)
        #expect(try cell(table, column: Self.c1, row: 0) === label)
        #expect(label.textField?.stringValue == "third")
        #expect(table.numberOfRows == 3)
    }

    @Test func columnReorderPreservesNativeSelectionAndLoadedValues() async throws {
        let model = try makeModel()
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let originalRows = model.tableInput.ordered
        let number = try cell(table, column: Self.c0, row: 1)
        let label = try cell(table, column: Self.c1, row: 1)
        try await waitUntil { !number.isLoading && !label.isLoading }
        table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        #expect(table.selectedRow == 1)

        model.reorderColumns([Self.c1, Self.c0])
        controller.update(showsRowNumbers: true)
        #expect(model.tableInput.ordered === originalRows)
        #expect(table.selectedRowIndexes == IndexSet(integer: 1))
        #expect(table.tableColumns[1].identifier.rawValue == "c1")
        // Check synchronously: a reset-and-refetch implementation cannot satisfy this.
        let movedLabel = try cell(table, column: Self.c1, row: 1)
        let movedNumber = try cell(table, column: Self.c0, row: 1)
        #expect(!movedLabel.isLoading && movedLabel.textField?.stringValue == "second")
        #expect(!movedNumber.isLoading && movedNumber.textField?.stringValue == "2")
    }

    @Test func changingRowCountRebindsValuesAndSourceRowsThroughEmptyResults() async throws {
        let model = try makeModel()
        let loader = TableSegmentLoader()
        let controller = DataTableController(model: model, loader: loader)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !initial.isLoading }
        #expect(table.numberOfRows == 3 && initial.textField?.stringValue == "1")

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 2)
        let first = try cell(table, column: Self.c0, row: 0)
        let second = try cell(table, column: Self.c0, row: 1)
        let label = try cell(table, column: Self.c1, row: 0)
        try await waitUntil { !first.isLoading && !second.isLoading && !label.isLoading }
        #expect(first.textField?.stringValue == "2" && second.textField?.stringValue == "3")
        #expect(label.textField?.stringValue == "second")
        #expect(loader.sourceRow(for: 0) == 1)
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)

        model.setFilter(Filter(condition: .range(min: "100", max: nil), includeNulls: false), column: Self.c0)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 0 && table.selectedRow == -1)
        #expect(table.rows(in: table.visibleRect).length == 0)
        #expect(loader.sourceRow(for: 0) == nil)

        model.clearFilters()
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 3)
        let restored = try cell(table, column: Self.c0, row: 0)
        let restoredLabel = try cell(table, column: Self.c1, row: 0)
        try await waitUntil { !restored.isLoading && !restoredLabel.isLoading }
        #expect(restored.textField?.stringValue == "1" && restoredLabel.textField?.stringValue == "first")
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        #expect(table.selectedRow == 0)
        #expect(loader.sourceRow(for: 0) == 0)
        #expect(model.errorMessage == nil)
    }

    @Test func scrollingBackAfterSortingNeverReusesOldOrderText() async throws {
        let csv = "number,label\n" + (0..<200).map { "\($0),row\($0)" }.joined(separator: "\n")
        let model = try makeModel(csv: csv)
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)

        func checkVisibleEdges() async throws {
            let visible = table.rows(in: table.visibleRect)
            try #require(visible.location != NSNotFound && visible.length > 0)
            let last = min(table.numberOfRows, visible.location + visible.length) - 1
            for row in [visible.location, last] {
                let expected = try [Self.c0, Self.c1].map {
                    try model.ordered.formatColumn(column: $0, start: UInt64(row), count: 1).cell(row: 0)
                }
                let number = try cell(table, column: Self.c0, row: row)
                let label = try cell(table, column: Self.c1, row: row)
                try await waitUntil { !number.isLoading && !label.isLoading }
                #expect(number.textField?.stringValue == expected[0])
                #expect(label.textField?.stringValue == expected[1])
            }
        }

        // Populate the original first screen before leaving it.
        try await checkVisibleEdges()
        table.scrollRowToVisible(120)
        controller.view.layoutSubtreeIfNeeded()
        table.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)
        #expect(table.rows(in: table.visibleRect).location > 0)
        try await checkVisibleEdges()

        model.setSortDirection(column: Self.c0, descending: true)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        table.layoutSubtreeIfNeeded()
        try await checkVisibleEdges()

        table.scrollRowToVisible(0)
        controller.view.layoutSubtreeIfNeeded()
        table.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)
        #expect(table.rows(in: table.visibleRect).location == 0)
        try await checkVisibleEdges()
        #expect(try cell(table, column: Self.c0, row: 0).textField?.stringValue == "199")
        #expect(model.errorMessage == nil)
    }

    @Test func columnLayoutChangeDuringViewportPreloadReloadsUpdatedColumnsWithoutRerunningQuery() async throws {
        let header = (0..<16).map { "c\($0)" }.joined(separator: ",")
        let rows = (0..<3).map { row in (0..<16).map { String(row * 100 + $0) }.joined(separator: ",") }
        let model = try makeModel(csv: header + "\n" + rows.joined(separator: "\n"))
        let original = model.ordered
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let controller = DataTableController(
            model: model, loader: gatedLoader(original: original, requests: requests.continuation))
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !initial.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setSortDirection(column: Self.c0, descending: true)
        let firstRead = try #require(await reads.next())
        let c15 = ColumnId(raw: 15)
        #expect(!firstRead.columns.contains(c15))
        let computedOrdered = firstRead.view

        // Hide off-viewport/unrelated columns during preload so c15 enters the viewport.
        model.setColumnsVisible((1..<14).map { ColumnId(raw: UInt32($0)) }, visible: false)
        firstRead.finish()
        let secondRead = try #require(await reads.next())
        #expect(secondRead.view === computedOrdered)
        #expect(secondRead.columns.contains(c15))
        secondRead.finish()

        try await waitUntil { !model.isBusy }
        #expect(model.ordered === computedOrdered)
        #expect(model.visibleColumns == [Self.c0, ColumnId(raw: 14), c15])
        let exposed = try cell(table, column: c15, row: 0)
        let sortedLead = try cell(table, column: Self.c0, row: 0)
        #expect(!sortedLead.isLoading && sortedLead.textField?.stringValue == "200")
        #expect(!exposed.isLoading && exposed.textField?.stringValue == "215")
    }

    @Test func failedQueryRestoresDraggedNativeTableColumns() async throws {
        let queryWorker = OperationQueue()
        queryWorker.isSuspended = true
        defer { queryWorker.isSuspended = false }
        let model = try makeModel(queryWorker: queryWorker)
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: Self.c0, row: 0)
        try await waitUntil { !initial.isLoading }
        #expect(controller.displayedColumns == [Self.c0, Self.c1])

        // Queue a query that will fail once the worker resumes.
        model.setFilter(Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: Self.c0)
        while queryWorker.operations.first(where: { !$0.isFinished && !$0.isCancelled }) == nil {
            await Task.yield()
        }

        // Simulate dragging column c1 ahead of c0 while the failing query is in flight.
        table.moveColumn(2, toColumn: 1)
        #expect(controller.displayedColumns == [Self.c1, Self.c0])
        #expect(model.draftVisibleColumns == [Self.c1, Self.c0])
        #expect(model.visibleColumns == [Self.c0, Self.c1])

        queryWorker.isSuspended = false
        try await waitUntil { !model.isBusy }
        #expect(model.errorMessage != nil)
        #expect(model.visibleColumns == [Self.c0, Self.c1])
        #expect(model.draftVisibleColumns == [Self.c0, Self.c1])
        #expect(controller.displayedColumns == [Self.c0, Self.c1])
    }

    private func makeModel(
        csv: String = "number,label\n1,first\n2,second\n3,third\n",
        queryWorker: OperationQueue? = nil
    ) throws -> DocumentModel {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("table-presentation-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        return DocumentModel(url: path, dataset: dataset, queryWorker: queryWorker)
    }

    private func prepareTable(_ controller: DataTableController) throws -> NSTableView {
        controller.loadViewIfNeeded()
        controller.view.frame = NSRect(x: 0, y: 0, width: 640, height: 240)
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        let scroll = try #require(controller.view.subviews.compactMap { $0 as? NSScrollView }.first)
        let table = try #require(scroll.documentView as? NSTableView)
        table.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)
        return table
    }

    private func cell(_ table: NSTableView, column: ColumnId, row: Int) throws -> MatchCellView {
        let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column.raw)"))
        #expect(index >= 0)
        return try #require(table.view(atColumn: index, row: row, makeIfNecessary: true) as? MatchCellView)
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw TablePresentationTimeout()
    }
}

private struct TablePresentationTimeout: Error {}

private struct PendingViewportRead: Sendable {
    let view: GewuOrderedView
    let keys: [BlockKey]
    let continuation: CheckedContinuation<TableSegments, Error>

    var start: Int { keys.map(\.start).min() ?? 0 }
    var columns: [ColumnId] { keys.compactMap(\.column) }

    func finish() {
        continuation.resume(with: Result { try TableSegments(reading: view, keys: keys) })
    }
}
