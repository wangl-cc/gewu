import AppKit
import Foundation
import Observation
import Testing
import os

@Suite @MainActor
struct QuerySpecTests {
    private static let c0 = ColumnId(raw: 0)
    private static let c1 = ColumnId(raw: 1)
    private static let c2 = ColumnId(raw: 2)

    @Test func queryEditsPublishOnlyTheirFinalSpec() throws {
        let model = try model()
        defer { model.stop() }
        let changes = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            _ = model.draftSorts
        } onChange: {
            changes.withLock { $0 += 1 }
        }

        model.cycleSort(column: Self.c0, multiple: false)
        let ascending = [ColumnSort(column: Self.c0, descending: false)]
        #expect(model.draftSorts == ascending)
        #expect(changes.withLock { $0 } == 1)

        withObservationTracking {
            _ = model.draftSorts
        } onChange: {
            changes.withLock { $0 += 1 }
        }
        model.setSortDirection(column: Self.c0, descending: false)
        #expect(changes.withLock { $0 } == 1, "No-op sort edit must not notify draftSorts")

        model.cycleSort(column: Self.c1, multiple: true)
        let multiple = [
            ColumnSort(column: Self.c0, descending: false),
            ColumnSort(column: Self.c1, descending: false),
        ]
        #expect(model.draftSorts == multiple)
        #expect(changes.withLock { $0 } == 2)

        withObservationTracking {
            _ = model.draftSorts
        } onChange: {
            changes.withLock { $0 += 1 }
        }
        model.clearSort()
        #expect(changes.withLock { $0 } == 3)
        withObservationTracking {
            _ = model.draftSorts
        } onChange: {
            changes.withLock { $0 += 1 }
        }
        model.clearSort()
        #expect(changes.withLock { $0 } == 3)
        #expect(model.draftSorts.isEmpty)
        #expect(!model.isBusy)
    }

    @Test func statusDescribesAppliedResultsIncludingFiltersThatKeepEveryRow() async throws {
        let model = try model()
        defer { model.stop() }
        #expect(model.status == "3 rows · 3 columns")
        model.setFilter(Filter(condition: .range(min: "1", max: nil), includeNulls: false), column: Self.c0)
        // A pending query must not describe the rows still on screen as already filtered.
        #expect(model.status == "3 rows · 3 columns")
        try await settled(model)
        #expect(model.status == "3 / 3 rows · 3 columns")
        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        try await settled(model)
        #expect(model.status == "1 / 3 rows · 3 columns")
        model.clearFilters()
        #expect(model.status == "1 / 3 rows · 3 columns")
        try await settled(model)
        #expect(model.status == "3 rows · 3 columns")
    }

    @Test func replacingSortColumnKeepsPriorityDirectionAndFilters() async throws {
        let model = try model()
        defer { model.stop() }
        let filter = Filter(condition: .range(min: "1", max: nil), includeNulls: true)
        model.setFilter(filter, column: Self.c0)
        model.setSortDirection(column: Self.c0, descending: true)
        model.setSortDirection(column: Self.c1, descending: false)
        model.replaceSortColumn(Self.c0, with: Self.c2)
        try await settled(model)
        #expect(model.viewer.appliedSorts.map(\.column) == [Self.c2, Self.c1])
        #expect(model.viewer.appliedSorts.first?.descending == true)
        #expect(model.appliedFilter(for: Self.c0) == filter)
        model.replaceSortColumn(Self.c2, with: Self.c1)
        #expect(model.draftSorts.map(\.column) == [Self.c2, Self.c1])
    }

    @Test func clearingQueryKeepsColumnArrangementAndIndependentFindAndInspector() async throws {
        let model = try model()
        defer { model.stop() }
        model.removeColumn(Self.c2)
        try await settled(model)
        model.reorderColumns([Self.c1, Self.c0])
        model.find.query = "1"
        model.inspectColumn(Self.c0)
        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        model.setSortDirection(column: Self.c1, descending: true)
        try await settled(model)
        model.clearFilters()
        model.clearSort()
        try await settled(model)
        #expect(model.ordered.rowCount() == 3)
        #expect(model.visibleColumns == [Self.c1, Self.c0])
        #expect(model.viewer.appliedSorts.isEmpty && model.appliedFilterColumns.isEmpty)
        #expect(model.find.query == "1" && model.inspectedColumn == Self.c0)
        model.inspectorPage = .chart
        model.focusSearch()
        #expect(model.inspectorPage == .chart && model.searchFocusRequest == 1)
    }

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
        #expect(model.visibleColumns == [Self.c1, Self.c0, Self.c2])
        controller.update(showsRowNumbers: true, fontSize: 12, rowHeight: 28, rowNumberBase: 0)
        #expect(controller.displayedColumns == [Self.c1, Self.c0, Self.c2])
        #expect(model.viewer.appliedSorts.isEmpty)
    }

    @Test func showAllRestoresHiddenColumnsWithoutRestoringTheirConditions() async throws {
        let model = try model()
        defer { model.stop() }
        model.reorderColumns([Self.c2, Self.c1, Self.c0])
        model.setSortDirection(column: Self.c0, descending: true)
        model.removeColumn(Self.c0)
        try await settled(model)
        model.showAllColumns()
        try await settled(model)
        #expect(model.visibleColumns == [Self.c2, Self.c1, Self.c0])
        #expect(model.appliedSort(for: Self.c0) == nil)
        #expect(model.appliedFilter(for: Self.c0) == nil)
    }

    @Test func removingOneAppliedFilterPreservesOtherFiltersSortingAndFindInput() async throws {
        let model = try model()
        defer { model.stop() }
        model.find.query = "x"
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: Self.c0)
        model.setFilter(Filter(condition: .exact(values: ["x"], exclude: false), includeNulls: false), column: Self.c2)
        model.setSortDirection(column: Self.c1, descending: true)
        try await settled(model)
        model.reorderColumns([Self.c2, Self.c0, Self.c1])
        model.setFilter(nil, column: Self.c2)
        try await settled(model)
        #expect(model.appliedFilterColumns == [Self.c0])
        #expect(model.ordered.rowCount() == 2)
        #expect(model.viewer.appliedSorts.map(\.column) == [Self.c1])
        #expect(model.find.query == "x")
        model.clearFilters()
        try await settled(model)
        #expect(model.ordered.rowCount() == 3 && model.appliedFilterColumns.isEmpty)
        #expect(model.viewer.appliedSorts.map(\.column) == [Self.c1] && model.find.query == "x")
    }

    private func model() throws -> DocumentModel {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-query-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "a,b,c\n1,2,x\n1,1,y\n2,0,z\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        return DocumentModel(url: path, dataset: dataset)
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
        model.cycleSort(column: Self.c0, multiple: false)
        model.cycleSort(column: Self.c1, multiple: true)
        #expect(model.draftSorts.map(\.column) == [Self.c0, Self.c1])
        model.reorderColumns([Self.c2, Self.c1, Self.c0])
        #expect(model.draftSorts.map(\.column) == [Self.c0, Self.c1])
        model.cycleSort(column: Self.c0, multiple: true)
        #expect(model.draftSort(for: Self.c0)?.descending == true)
        model.cycleSort(column: Self.c0, multiple: true)
        #expect(model.draftSorts.map(\.column) == [Self.c1])
        #expect(model.sortPriority(for: Self.c1, in: model.draftSorts) == 0)
        model.cycleSort(column: Self.c2, multiple: false)
        #expect(model.draftSorts.map(\.column) == [Self.c2])
        model.clearSort()
        try await settled(model)
        #expect(model.visibleColumns == [Self.c2, Self.c1, Self.c0])
        #expect(
            try model.ordered.rows(start: 0, count: 10, columns: [Self.c0]).sourceRows == [
                0, 1, 2,
            ])
    }

    @Test func confirmedHideClearsConditionsAndFindUsesRemainingColumns() async throws {
        let model = try model()
        defer { model.stop() }
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: Self.c0)
        model.setSortDirection(column: Self.c0, descending: true)
        try await settled(model)
        #expect(model.ordered.rowCount() == 2)
        model.removeColumn(Self.c0)
        try await settled(model)
        #expect(model.ordered.rowCount() == 3)
        #expect(model.viewer.appliedSorts.isEmpty)
        #expect(model.appliedFilterColumns.isEmpty)
        model.find.query = "1"
        model.updateFind()
        for _ in 0..<500 {
            if model.find.hasMatches != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(model.find.currentMatch?.column == Self.c1)
        model.showColumn(Self.c0)
        #expect(model.visibleColumns.last == Self.c0)
        #expect(model.appliedFilter(for: Self.c0) == nil)
        #expect(model.appliedSort(for: Self.c0) == nil)
        try await settled(model)
    }

    @Test func hidingInspectedSelectedAndChartColumnsFallsBackToRemainingVisibleColumns() async throws {
        let model = try model()
        defer { model.stop() }
        model.selectedColumn = Self.c0
        model.inspectedColumn = Self.c0
        model.chartX = Self.c0
        model.chartY = Self.c0

        model.removeColumn(Self.c0)
        try await settled(model)
        #expect(model.selectedColumn == Self.c1)
        #expect(model.inspectedColumn == Self.c1)
        #expect(model.chartX == nil)
        #expect(model.chartY == Self.c1)

        // Hiding the remaining plottable column leaves chartY as nil while keeping c2 selected/inspected.
        model.removeColumn(Self.c1)
        try await settled(model)
        #expect(model.selectedColumn == Self.c2)
        #expect(model.inspectedColumn == Self.c2)
        #expect(model.chartY == nil)

        // Hiding the last visible column clears selectedColumn and inspectedColumn.
        model.removeColumn(Self.c2)
        try await settled(model)
        #expect(model.selectedColumn == nil)
        #expect(model.inspectedColumn == nil)

        // Hidden columns cannot be inspected; restoring columns recovers selection and inspector target.
        model.inspectColumn(Self.c0)
        #expect(model.inspectedColumn == nil)
        model.showAllColumns()
        try await settled(model)
        #expect(model.selectedColumn == Self.c0)
        #expect(model.inspectedColumn == Self.c0)
    }

    @Test func filtersStayInCanonicalSchemaOrderRegardlessOfEditSequence() async throws {
        let model = try model()
        defer { model.stop() }
        let c0Filter = Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false)
        let c2Filter = Filter(condition: .exact(values: ["x"], exclude: false), includeNulls: false)

        model.setFilter(c2Filter, column: Self.c2)
        model.setFilter(c0Filter, column: Self.c0)
        try await settled(model)
        let originalFiltered = model.filtered
        #expect(model.viewer.appliedFilters.map(\.column) == [Self.c0, Self.c2])

        // Re-applying the same filters in the opposite order produces the exact same canonical list.
        model.setFilter(nil, column: Self.c0)
        model.setFilter(c0Filter, column: Self.c0)
        #expect(model.draftFilters.map(\.column) == [Self.c0, Self.c2])
        #expect(!model.isBusy && model.filtered === originalFiltered)
    }

    @Test func revertingPendingFilterWhileVisibilityChangedAppliesColumnsSynchronously() async throws {
        let model = try model()
        defer { model.stop() }
        model.selectedDisplayRow = 1
        model.selectedSourceRow = 1

        let c0Filter = Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false)
        model.setFilter(c0Filter, column: Self.c0)
        model.removeColumn(Self.c2)
        model.setFilter(nil, column: Self.c0)

        #expect(!model.isBusy)
        #expect(model.visibleColumns == [Self.c0, Self.c1])
        #expect(model.selectedDisplayRow == 1 && model.selectedSourceRow == 1)
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

    @Test func valueSelectionTreatsMissingAsAValueAndResetClearsEveryDraft() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-null-selection-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nalpha,1\n,2\nMissing (null),3\nbeta,4\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        defer { model.stop() }
        var draft = ColumnFilterDraft(nil)
        draft.selectAllValues(false)
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(model.ordered.rowCount() == 0)

        // Reopening a zero-row result can recover through the complete facet.
        let choices = try await model.viewer.filterChoices(Self.c0)
        #expect(choices.values?.count == 3 && choices.nullCount == 1)
        var reopened = ColumnFilterDraft(model.appliedFilter(for: Self.c0))
        reopened.enterValues()
        reopened.selectAllValues(true)
        #expect(reopened.filter(mode: .exact) == nil)

        draft.includesMissing = true
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(try model.ordered.rows(start: 0, count: 10, columns: [Self.c0]).sourceRows == [1])

        // A null-only filter reopens as an editable exact selection.
        draft = ColumnFilterDraft(draft.filter(mode: .exact))
        draft.enterValues()
        draft.includesMissing = false
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(model.ordered.rowCount() == 0)
        draft.includesMissing = true
        draft.setValue(LiteralTextValue(value: "Missing (null)"), selected: true)
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(try model.ordered.rows(start: 0, count: 10, columns: [Self.c0]).sourceRows == [1, 2])

        draft.selectAllValues(true)
        #expect(draft.filter(mode: .exact) == nil)
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(model.ordered.rowCount() == 4)

        draft.text.text = "alpha"
        draft.range.minimum = "3"
        draft.range.lowerEdited = true
        draft.nullMode = .only
        draft.enterValues()
        #expect(!draft.exact.contains(LiteralTextValue(value: "alpha")))
        #expect(draft.includesMissing)
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        draft.reset()
        #expect(draft.filter(mode: .exact) == nil)
        #expect(draft.filter(mode: .text) == nil)
        #expect(draft.filter(mode: .range) == nil)
        // Reset edits the draft, not the applied view.
        #expect(model.ordered.rowCount() == 1)
        model.setFilter(draft.filter(mode: .exact), column: Self.c0)
        try await settled(model)
        #expect(model.ordered.rowCount() == 4)
    }

    @Test func bulkVisibilityClearsOnlyHiddenConditionsAndPreservesRemainingOrder() async throws {
        let model = try model()
        defer { model.stop() }
        model.reorderColumns([Self.c2, Self.c1, Self.c0])
        model.setFilter(Filter(condition: .exact(values: ["x"], exclude: false), includeNulls: false), column: Self.c2)
        model.setSortDirection(column: Self.c1, descending: true)
        model.setSortDirection(column: Self.c0, descending: false)
        try await settled(model)
        model.setColumnsVisible([Self.c2, Self.c1], visible: false)
        try await settled(model)
        #expect(model.visibleColumns == [Self.c0])
        #expect(model.ordered.rowCount() == 3)
        #expect(model.sortPriority(for: Self.c0, in: model.viewer.appliedSorts) == 0)
        model.setColumnsVisible([Self.c2, Self.c1], visible: true)
        try await settled(model)
        #expect(model.visibleColumns == [Self.c0, Self.c1, Self.c2])
        #expect(model.appliedFilter(for: Self.c1) == nil && model.appliedFilter(for: Self.c2) == nil)
        #expect(model.appliedSort(for: Self.c1) == nil && model.appliedSort(for: Self.c2) == nil)
    }

    @Test func failedReplacementKeepsTheWindowViewAndSpec() async throws {
        let model = try model()
        defer { model.stop() }
        model.setFilter(Filter(condition: .exact(values: ["1"], exclude: false), includeNulls: false), column: Self.c0)
        try await settled(model)
        let acceptedFilters = model.viewer.appliedFilters
        let acceptedSorts = model.viewer.appliedSorts
        let ordered = model.ordered
        model.setFilter(Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: Self.c0)
        try await settled(model)
        #expect(model.errorMessage != nil)
        #expect(model.ordered === ordered)
        #expect(model.viewer.appliedFilters == acceptedFilters && model.draftFilters == acceptedFilters)
        #expect(model.viewer.appliedSorts == acceptedSorts && model.draftSorts == acceptedSorts)
        #expect(try model.ordered.rows(start: 0, count: 3, columns: [Self.c0]).nRows == 2)
    }

    @Test func windowsSharingDatasetOwnIndependentQueriesAndCancellation() async throws {
        let first = try model()
        let second = DocumentModel(url: first.url, dataset: first.dataset)
        defer {
            first.stop()
            second.stop()
        }
        let original = first.ordered
        first.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: Self.c0)
        second.setFilter(Filter(condition: .exact(values: ["y"], exclude: false), includeNulls: false), column: Self.c2)
        try await settled(first)
        try await settled(second)
        #expect(try first.ordered.rows(start: 0, count: 3, columns: [Self.c0]).sourceRows == [2])
        #expect(try second.ordered.rows(start: 0, count: 3, columns: [Self.c0]).sourceRows == [1])
        first.refresh(debounce: true)
        first.stop()
        #expect(try second.filtered.countNulls(column: Self.c0) == 0)
        #expect(original.rowCount() == 3)
        #expect(first.viewer.appliedFilters != second.viewer.appliedFilters)
    }
}
