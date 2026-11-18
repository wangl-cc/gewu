import AppKit
import Combine
import Foundation
import Testing

@Suite @MainActor
struct TablePresentationTests {
    @Test func replacementKeepsOldRowsUntilEveryVisibleValueIsReady() async throws {
        let longLabel = "second" + String(repeating: "x", count: 1024)
        let model = try makeModel(csv: "number,label\n1,first\n2,\(longLabel)\n3,third\n")
        let original = model.view
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let loader = gatedLoader(original: original, requests: requests.continuation, byteLimit: 256)
        let controller = DataTableController(model: model, loader: loader)
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let first = try cell(table, column: 0, row: 0)
        try await waitUntil { !first.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: 0)
        let pending = try #require(await reads.next())
        controller.update(showsRowNumbers: true)
        #expect(model.isBusy && model.view === original)
        #expect(model.appliedSpec.columns[0].filter == nil)
        #expect(table.numberOfRows == 3 && first.textField?.stringValue == "1" && !first.isLoading)
        pending.finish()
        try await waitUntil { !model.isBusy }

        // No wait for cell delivery after publication: all values, including
        // ones larger than the cache budget, must already be installed.
        #expect(table.numberOfRows == 2)
        let number = try cell(table, column: 0, row: 0)
        let label = try cell(table, column: 1, row: 0)
        #expect(!number.isLoading && number.textField?.stringValue == "2")
        #expect(!label.isLoading && label.textField?.stringValue == longLabel)
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        #expect(model.selectedSourceRow == 1)

        // A later presentation update must keep the committed rows and uncached
        // large values, without clearing the native selection or refetching.
        let committed = model.view
        controller.update(showsRowNumbers: false, fontSize: 16, rowHeight: 32, rowNumberBase: 1)
        #expect(model.view === committed && table.numberOfRows == 2)
        #expect(table.selectedRow == 0 && model.selectedSourceRow == 1)
        #expect(!label.isLoading && label.textField?.stringValue == longLabel)
    }

    @Test func supersededViewportAndReadFailureCannotReplaceTheDisplayedData() async throws {
        let model = try makeModel()
        let original = model.view
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let controller = DataTableController(
            model: model, loader: gatedLoader(original: original, requests: requests.continuation))
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let first = try cell(table, column: 0, row: 0)
        try await waitUntil { !first.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: 0)
        let older = try #require(await reads.next())
        model.setFilter(Filter(condition: .range(min: "3", max: nil), includeNulls: false), column: 0)
        let newer = try #require(await reads.next())
        newer.finish()
        try await waitUntil { !model.isBusy }
        let committed = model.view
        #expect(table.numberOfRows == 1 && first.textField?.stringValue == "3" && !first.isLoading)
        older.finish()

        model.clearFilters()
        let failing = try #require(await reads.next())
        failing.continuation.resume(throwing: TablePresentationTimeout())
        try await waitUntil { !model.isBusy }
        #expect(model.view === committed && table.numberOfRows == 1)
        #expect(first.textField?.stringValue == "3" && !first.isLoading)
        #expect(model.errorMessage != nil)

        model.clearFilters()
        let closing = try #require(await reads.next())
        model.stop()
        closing.finish()
        for _ in 0..<10 { await Task.yield() }
        #expect(model.view === committed && table.numberOfRows == 1)
    }

    @Test func scrollingDuringPreloadLoadsTheNewViewportBeforeCommit() async throws {
        let csv = "number,label\n" + (0..<300).map { "\($0),row\($0)" }.joined(separator: "\n")
        let model = try makeModel(csv: csv)
        let original = model.view
        let requests = AsyncStream<PendingViewportRead>.makeStream()
        let controller = DataTableController(
            model: model, loader: gatedLoader(original: original, requests: requests.continuation))
        model.table = controller
        defer {
            model.stop()
            requests.continuation.finish()
        }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: 0, row: 0)
        try await waitUntil { !initial.isLoading }
        var reads = requests.stream.makeAsyncIterator()

        model.setSortDirection(column: 0, descending: true)
        let firstRead = try #require(await reads.next())
        #expect(firstRead.start == 0)
        table.scrollRowToVisible(180)
        controller.view.layoutSubtreeIfNeeded()
        controller.update(showsRowNumbers: true)
        firstRead.finish()
        let movedRead = try #require(await reads.next())
        #expect(movedRead.start >= 128)
        #expect(model.view === original, "An obsolete viewport cannot commit the new View")
        movedRead.finish()
        try await waitUntil { !model.isBusy }
        let visible = table.rows(in: table.visibleRect)
        #expect(visible.location > 100)
        for row in visible.location..<(visible.location + visible.length) {
            let number = try cell(table, column: 0, row: row)
            #expect(!number.isLoading && number.textField?.stringValue == String(299 - row))
        }
    }

    private func gatedLoader(
        original: GewuView, requests: AsyncStream<PendingViewportRead>.Continuation, byteLimit: Int = 8 * 1024 * 1024
    ) -> TableRowLoader {
        TableRowLoader(cache: RowCache(byteLimit: byteLimit)) { view, start, columns in
            if view === original {
                return try view.rows(start: UInt64(start), count: UInt32(RowCache.blockSize), columns: columns)
            }
            return try await withCheckedThrowingContinuation { continuation in
                requests.yield(.init(view: view, start: start, columns: columns, continuation: continuation))
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
        let initial = try cell(table, column: 0, row: 0)
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
            let columns = indexes.map { UInt32(table.tableColumns[$0].identifier.rawValue.dropFirst())! }
            let expected = try model.view.rows(
                start: UInt64(visible.location), count: UInt32(visible.length), columns: columns)
            for (r, row) in (visible.location..<(visible.location + visible.length)).enumerated() {
                for (c, column) in columns.enumerated() {
                    let value = try cell(table, column: Int(column), row: row)
                    #expect(!value.isLoading && value.textField?.stringValue == expected.cells[r * columns.count + c])
                }
            }
        }

        model.setFilter(Filter(condition: .range(min: "2900", max: nil), includeNulls: false), column: 0)
        try await waitUntil { !model.isBusy }
        #expect(table.numberOfRows == 10)
        try checkReady()
        // Removing the filtered column also restores all rows and exposes columns
        // that were outside the old horizontal viewport.
        model.setColumnsVisible([0, 1, 2, 3], visible: false)
        try await waitUntil { !model.isBusy }
        #expect(table.numberOfRows == 300)
        try checkReady()
    }

    @Test func tableInputPublishesOnlyRelevantWindowChanges() throws {
        let model = try makeModel()
        defer { model.stop() }
        var updates = 0
        let observation = model.tablePresentation.$input.dropFirst().sink { _ in updates += 1 }

        model.inspectorPage = .chart
        model.chartX = 0
        model.chartY = 1
        model.selectedDisplayRow = 1
        model.selectedSourceRow = 1
        model.showsSorting = true
        model.setSortDirection(column: 0, descending: true)
        #expect(model.isBusy)
        #expect(updates == 0, "Pending queries and unrelated window state are not table inputs")
        model.clearSort()
        #expect(!model.isBusy && updates == 0)

        model.selectedColumn = 1
        #expect(updates == 1)
        #expect(model.tablePresentation.input.selectedColumn == 1)
        model.selectedColumn = 1
        #expect(updates == 1, "Assigning an unchanged header selection must not republish it")
        withExtendedLifetime(observation) {}
    }

    @Test func unchangedAndCancelledBeforeStartQueriesKeepTheAppliedSnapshot() async throws {
        let model = try makeModel()
        defer { model.stop() }
        let original = model.view
        model.clearSort()
        model.setFilter(nil, column: 0)
        #expect(!model.isBusy && model.view === original)

        // Both edits happen in one main-actor turn, before the query Task can run.
        model.setSortDirection(column: 0, descending: true)
        #expect(model.isBusy)
        model.clearSort()
        #expect(!model.isBusy && model.view === original)
        for _ in 0..<10 {
            await Task.yield()
            #expect(model.view === original && !model.isBusy)
        }

        // Let cancelled Tasks drain, then use a real query as a worker barrier.
        model.setSortDirection(column: 0, descending: false)
        try await waitUntil { !model.isBusy }
        #expect(model.appliedSpec.columns[0].sort?.descending == false)
        #expect(try model.view.rows(start: 0, count: 3, columns: [0]).cells == ["1", "2", "3"])
        model.clearSort()
        try await waitUntil { !model.isBusy }
        #expect(model.appliedSpec.columns.allSatisfy { $0.sort == nil })
        #expect(try model.view.rows(start: 0, count: 3, columns: [0]).cells == ["1", "2", "3"])
        #expect(model.errorMessage == nil)
    }

    @Test func nativeCellsSurviveFindDecorationAndSameSizeSorting() async throws {
        let model = try makeModel()
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let number = try cell(table, column: 0, row: 0)
        let label = try cell(table, column: 1, row: 0)
        try await waitUntil { !number.isLoading && !label.isLoading }
        #expect(number.textField?.stringValue == "1")

        model.find.query = "first"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }
        model.find.requestHighlight(start: 0, count: 3)
        try await waitUntil { model.find.highlight != nil }
        controller.update(showsRowNumbers: true)
        #expect(try cell(table, column: 1, row: 0) === label)
        #expect(label.currentMatch)
        #expect(
            label.textField?.attributedStringValue.attribute(.backgroundColor, at: 0, effectiveRange: nil) as? NSColor
                == .findHighlightColor)

        model.find.query = ""
        model.updateFind()
        controller.update(showsRowNumbers: true)
        #expect(try cell(table, column: 1, row: 0) === label)
        #expect(!label.currentMatch)

        model.setSortDirection(column: 0, descending: true)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        try await waitUntil { !number.isLoading && number.textField?.stringValue == "3" }
        #expect(try cell(table, column: 0, row: 0) === number)
        #expect(try cell(table, column: 1, row: 0) === label)
        #expect(label.textField?.stringValue == "third")
        #expect(table.numberOfRows == 3)
    }

    @Test func columnReorderPreservesNativeSelectionAndLoadedValues() async throws {
        let model = try makeModel()
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let originalRows = model.tablePresentation.input.rows
        let number = try cell(table, column: 0, row: 1)
        let label = try cell(table, column: 1, row: 1)
        try await waitUntil { !number.isLoading && !label.isLoading }
        table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        #expect(model.selectedDisplayRow == 1)
        #expect(model.selectedSourceRow == 1)

        model.reorderColumns([1, 0])
        controller.update(showsRowNumbers: true)
        #expect(model.tablePresentation.input.rows === originalRows)
        #expect(table.selectedRowIndexes == IndexSet(integer: 1))
        #expect(model.selectedDisplayRow == 1 && model.selectedSourceRow == 1)
        #expect(table.tableColumns[1].identifier.rawValue == "c1")
        // Check synchronously: a reset-and-refetch implementation cannot satisfy this.
        let movedLabel = try cell(table, column: 1, row: 1)
        let movedNumber = try cell(table, column: 0, row: 1)
        #expect(!movedLabel.isLoading && movedLabel.textField?.stringValue == "second")
        #expect(!movedNumber.isLoading && movedNumber.textField?.stringValue == "2")
    }

    @Test func changingRowCountRebindsValuesAndSourceRowsThroughEmptyResults() async throws {
        let model = try makeModel()
        let controller = DataTableController(model: model)
        model.table = controller
        defer { model.stop() }
        let table = try prepareTable(controller)
        let initial = try cell(table, column: 0, row: 0)
        try await waitUntil { !initial.isLoading }
        #expect(table.numberOfRows == 3 && initial.textField?.stringValue == "1")

        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: 0)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 2)
        let first = try cell(table, column: 0, row: 0)
        let second = try cell(table, column: 0, row: 1)
        let label = try cell(table, column: 1, row: 0)
        try await waitUntil { !first.isLoading && !second.isLoading && !label.isLoading }
        #expect(first.textField?.stringValue == "2" && second.textField?.stringValue == "3")
        #expect(label.textField?.stringValue == "second")
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        #expect(model.selectedDisplayRow == 0 && model.selectedSourceRow == 1)

        model.setFilter(Filter(condition: .range(min: "100", max: nil), includeNulls: false), column: 0)
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 0 && table.selectedRow == -1)
        #expect(table.rows(in: table.visibleRect).length == 0)
        #expect(model.selectedDisplayRow == nil && model.selectedSourceRow == nil)

        model.clearFilters()
        try await waitUntil { !model.isBusy }
        controller.update(showsRowNumbers: true)
        controller.view.layoutSubtreeIfNeeded()
        #expect(table.numberOfRows == 3)
        let restored = try cell(table, column: 0, row: 0)
        let restoredLabel = try cell(table, column: 1, row: 0)
        try await waitUntil { !restored.isLoading && !restoredLabel.isLoading }
        #expect(restored.textField?.stringValue == "1" && restoredLabel.textField?.stringValue == "first")
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        #expect(model.selectedDisplayRow == 0 && model.selectedSourceRow == 0)
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
                let expected = try model.view.rows(start: UInt64(row), count: 1, columns: [0, 1])
                let number = try cell(table, column: 0, row: row)
                let label = try cell(table, column: 1, row: row)
                try await waitUntil { !number.isLoading && !label.isLoading }
                #expect(number.textField?.stringValue == expected.cells[0])
                #expect(label.textField?.stringValue == expected.cells[1])
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

        model.setSortDirection(column: 0, descending: true)
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
        #expect(try cell(table, column: 0, row: 0).textField?.stringValue == "199")
        #expect(model.errorMessage == nil)
    }

    private func makeModel(csv: String = "number,label\n1,first\n2,second\n3,third\n") throws -> DocumentModel {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("table-presentation-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        return DocumentModel(url: path, dataset: dataset, view: dataset.view())
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

    private func cell(_ table: NSTableView, column: Int, row: Int) throws -> MatchCellView {
        let index = table.column(withIdentifier: NSUserInterfaceItemIdentifier("c\(column)"))
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
    let view: GewuView
    let start: Int
    let columns: [UInt32]
    let continuation: CheckedContinuation<RowBlock, Error>

    func finish() {
        continuation.resume(
            with: Result {
                try view.rows(start: UInt64(start), count: UInt32(RowCache.blockSize), columns: columns)
            })
    }
}
