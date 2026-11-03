import AppKit
import Foundation
import Testing

@Suite @MainActor
struct QuerySpecTests {
    @Test func nativeColumnDragAllowsPreflightAndKeepsTheRowGutterFixed() throws {
        let model = try model()
        let controller = DataTableController(model: model)
        defer {
            controller.stopLoading()
            model.stop()
        }
        let scroll = try #require(controller.view.subviews.compactMap { $0 as? NSScrollView }.first)
        let table = try #require(scroll.documentView as? NSTableView)
        controller.update(showsRowNumbers: true, fontSize: 12, rowHeight: 28, rowNumberBase: 0)
        #expect(table.allowsColumnReordering)
        #expect(controller.tableView(table, shouldReorderColumn: 2, toColumn: -1))
        #expect(controller.tableView(table, shouldReorderColumn: 2, toColumn: 1))
        #expect(!controller.tableView(table, shouldReorderColumn: 2, toColumn: 0))
        #expect(!controller.tableView(table, shouldReorderColumn: 0, toColumn: -1))
        #expect(!controller.tableView(table, shouldReorderColumn: 0, toColumn: 2))

        table.moveColumn(2, toColumn: 1)
        #expect(model.appliedSpec.columns.map(\.column) == [1, 0, 2])
        controller.update(showsRowNumbers: true, fontSize: 12, rowHeight: 28, rowNumberBase: 0)
        #expect(controller.displayedColumns == [1, 0, 2])
        #expect(model.sortedColumns.isEmpty)
    }

    @Test func showAllRestoresHiddenColumnsWithoutRestoringTheirConditions() async throws {
        let model = try model()
        defer { model.stop() }
        model.reorderColumns([2, 1, 0])
        model.setSortDirection(column: 0, descending: true)
        model.removeColumn(0)
        try await settled(model)
        model.showAllColumns()
        try await settled(model)
        #expect(model.appliedSpec.columns.map(\.column) == [2, 1, 0])
        #expect(model.appliedSpec.columns.last?.sort == nil)
        #expect(model.appliedSpec.columns.last?.filter == nil)
    }

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
}
