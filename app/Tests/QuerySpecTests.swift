import AppKit
import Foundation
import Testing

@Suite @MainActor
struct QuerySpecTests {
    @Test func removingOneAppliedFilterPreservesOtherFiltersSortingAndFindInput() async throws {
        let model = try model()
        defer { model.stop() }
        model.find.query = "x"
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: 0)
        model.setFilter(Filter(condition: .exact(values: ["x"], exclude: false), includeNulls: false), column: 2)
        model.setSortDirection(column: 1, descending: true)
        try await settled(model)
        model.reorderColumns([2, 0, 1])
        model.setFilter(nil, column: 2)
        try await settled(model)
        #expect(model.appliedFilterColumns == [0])
        #expect(model.view.rowCount() == 2)
        #expect(model.sortedColumns.map(\.column) == [1])
        #expect(model.find.query == "x")
        model.clearFilters()
        try await settled(model)
        #expect(model.view.rowCount() == 3 && model.appliedFilterColumns.isEmpty)
        #expect(model.sortedColumns.map(\.column) == [1] && model.find.query == "x")
    }

    private func model() throws -> DocumentModel {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-query-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "a,b,c\n1,2,x\n1,1,y\n2,0,z\n".write(to: path, atomically: true, encoding: .utf8)
        return DocumentModel(url: path, dataset: try GewuDataset.open(path: path.path))
    }

    private func settled(_ model: DocumentModel) async throws {
        for _ in 0..<500 {
            if !model.isBusy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("Query did not settle")
    }

    @Test func sortingCyclesAndPrioritiesRemainIndependentOfDisplayOrder() async throws {
        let model = try model()
        defer { model.stop() }
        model.cycleSort(column: 0, multiple: false)
        model.cycleSort(column: 1, multiple: true)
        #expect(model.sortedColumns.map(\.column) == [0, 1])
        model.reorderColumns([2, 1, 0])
        #expect(model.sortedColumns.map(\.column) == [0, 1])
        model.cycleSort(column: 0, multiple: true)
        #expect(model.sortedColumns[0].sort?.descending == true)
        model.cycleSort(column: 0, multiple: true)
        #expect(model.sortedColumns.map(\.column) == [1])
        #expect(model.sortedColumns[0].sort?.priority == 0)
        model.cycleSort(column: 2, multiple: false)
        #expect(model.sortedColumns.map(\.column) == [2])
        model.clearSort()
        try await settled(model)
        #expect(model.appliedSpec.columns.map(\.column) == [2, 1, 0])
        #expect(
            try model.view.rows(start: 0, count: 10, columns: [0]).sourceRows == [
                0, 1, 2,
            ])
    }

    @Test func confirmedHideClearsConditionsAndFindUsesRemainingColumns() async throws {
        let model = try model()
        defer { model.stop() }
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: 0)
        model.setSortDirection(column: 0, descending: true)
        try await settled(model)
        #expect(model.view.rowCount() == 2)
        model.removeColumn(0)
        try await settled(model)
        #expect(model.view.rowCount() == 3)
        #expect(model.sortedColumns.isEmpty)
        #expect(model.appliedFilterColumns.isEmpty)
        model.find.query = "1"
        model.updateFind()
        for _ in 0..<500 {
            if model.find.hasMatches != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(model.find.currentMatch?.column == 1)
        model.showColumn(0)
        #expect(model.spec.columns.last?.column == 0)
        #expect(model.spec.columns.last?.filter == nil)
        #expect(model.spec.columns.last?.sort == nil)
        try await settled(model)
    }

    @Test func shiftHeaderClickUsesExplicitMultiSortCallback() throws {
        let table = NSTableView(frame: NSRect(x: 0, y: 0, width: 120, height: 100))
        let column = NSTableColumn(identifier: .init("value"))
        column.width = 120
        table.addTableColumn(column)
        let header = FilterHeaderView(frame: NSRect(x: 0, y: 0, width: 120, height: 32))
        table.headerView = header
        var clicked: Int?
        header.onShiftSort = { clicked = $0 }
        let event = try #require(
            NSEvent.mouseEvent(
                with: .leftMouseDown, location: NSPoint(x: 20, y: 10), modifierFlags: .shift,
                timestamp: 0, windowNumber: 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        header.mouseDown(with: event)
        #expect(clicked == 0)
    }

    @Test func exactDraftPreservesBytesAndNullsAreIndependent() {
        let composed = LiteralTextValue(value: "é")
        let decomposed = LiteralTextValue(value: "e\u{301}")
        #expect(composed != decomposed)
        var draft = ExactFilterDraft(nil)
        draft.set(composed, selected: false)
        #expect(!draft.contains(composed) && draft.contains(decomposed))
        let filter = NullMode.exclude.applying(to: draft.condition)
        #expect(filter?.includeNulls == false)
        if case .exact(let values, let exclude) = filter?.condition {
            #expect(exclude && values.count == 1)
        } else {
            Issue.record("Expected exact selection")
        }
        let onlyNull = NullMode.only.applying(to: draft.condition)
        #expect(onlyNull?.includeNulls == true)
        if case .exact(let values, let exclude) = onlyNull?.condition {
            #expect(values.isEmpty && !exclude)
        } else {
            Issue.record("Expected null-only selection")
        }
        draft.all(true)
        #expect(NullMode.include.applying(to: draft.condition) == nil)
    }

    @Test func failedReplacementKeepsTheWindowViewAndSpec() async throws {
        let model = try model()
        defer { model.stop() }
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: 0)
        try await settled(model)
        let accepted = model.appliedSpec
        let view = model.view
        model.setFilter(Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: 0)
        try await settled(model)
        #expect(model.errorMessage != nil)
        #expect(model.view === view)
        #expect(model.appliedSpec == accepted && model.spec == accepted)
        #expect(try model.view.rows(start: 0, count: 3, columns: [0]).nRows == 2)
    }

    @Test func windowsSharingDatasetOwnIndependentQueriesAndCancellation() async throws {
        let first = try model()
        let second = DocumentModel(url: first.url, dataset: first.dataset)
        defer {
            first.stop()
            second.stop()
        }
        let original = first.view
        first.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: 0)
        second.setFilter(Filter(condition: .exact(values: ["y"], exclude: false), includeNulls: false), column: 2)
        try await settled(first)
        try await settled(second)
        #expect(try first.view.rows(start: 0, count: 3, columns: [0]).sourceRows == [2])
        #expect(try second.view.rows(start: 0, count: 3, columns: [0]).sourceRows == [1])
        first.refresh(debounce: true)
        first.stop()
        #expect(try second.view.countNulls(column: 0, cancellation: Cancellation()) == 0)
        #expect(original.rowCount() == 3)
        #expect(first.appliedSpec != second.appliedSpec)
    }

    @Test func candidateContextTracksTheVisibleView() async throws {
        let document = try model()
        defer { document.stop() }
        let choices = TextChoicesModel()
        defer { choices.cancel() }
        let original = document.view
        await choices.load(view: original, column: 2, query: "", after: nil)
        #expect(choices.page?.values.map(\.value) == ["x", "y", "z"])
        document.setFilter(Filter(condition: .exact(values: ["y"], exclude: false), includeNulls: false), column: 2)
        try await settled(document)
        await choices.load(view: document.view, column: 2, query: "", after: nil)
        #expect(choices.page?.values.map(\.value) == ["y"])
        await choices.load(view: original, column: 2, query: "", after: nil)
        #expect(choices.page?.values.map(\.value) == ["x", "y", "z"])
    }

    @Test func cancellingCandidateRequestDoesNotCloseReplacement() async throws {
        let document = try model()
        defer { document.stop() }
        let choices = TextChoicesModel()
        defer { choices.cancel() }
        let old = Task {
            await choices.load(view: document.view, column: 2, query: "x", after: nil)
        }
        await Task.yield()
        old.cancel()
        choices.pause()
        await choices.load(view: document.view, column: 2, query: "y", after: nil)
        await old.value
        #expect(choices.page?.values.map(\.value) == ["y"])
        #expect(!choices.loading && choices.error == nil)
    }
}
