import Foundation
import GewuKit
import Observation
import Testing
import os

@Suite @MainActor
struct QuerySpecTests {
    private static let csv = "a,b,c\n1,2,x\n1,1,y\n2,0,z\n"

    private func model(alerts: Alerts = Alerts()) async throws -> TableModel {
        try await TestTables.model(Self.csv, alerts: alerts)
    }

    private func settled(_ model: TableModel) async {
        await eventually { !model.isUpdating }
        #expect(!model.isUpdating, "Query did not settle")
    }

    private func exact(_ value: String) -> Filter {
        Filter(condition: .exact(values: [value], exclude: false), includeNulls: false)
    }

    @Test func queryEditsPublishOnlyTheirFinalDraft() async throws {
        let model = try await model()
        defer { model.stop() }
        let a = model.id("a")
        let b = model.id("b")
        let changes = OSAllocatedUnfairLock(initialState: 0)
        func track() {
            withObservationTracking {
                _ = model.draft
            } onChange: {
                changes.withLock { $0 += 1 }
            }
        }
        track()
        model.cycleSort(column: a, multiple: false)
        #expect(model.draft.sorts == [ColumnSort(column: a, descending: false)])
        #expect(changes.withLock { $0 } == 1)

        track()
        model.setSortDirection(column: a, descending: false)
        #expect(changes.withLock { $0 } == 1, "A no-op sort edit must not notify")

        model.cycleSort(column: b, multiple: true)
        #expect(model.draft.sorts.map(\.column) == [a, b])
        #expect(changes.withLock { $0 } == 2)
        track()
        model.clearSorts()
        #expect(changes.withLock { $0 } == 3)
        track()
        model.clearSorts()
        #expect(changes.withLock { $0 } == 3 && model.draft.sorts.isEmpty)
        #expect(!model.isUpdating, "Returning to the displayed query cancels the pending one")
    }

    @Test func statusDescribesDisplayedRowsIncludingFiltersThatKeepEveryRow() async throws {
        let model = try await model()
        defer { model.stop() }
        let a = model.id("a")
        #expect(rowSummary(model) == "3 rows · 3 columns")
        model.setFilter(Filter(condition: .range(min: "1", max: nil), includeNulls: false), column: a)
        #expect(rowSummary(model) == "3 rows · 3 columns", "A pending query does not describe shown rows")
        await settled(model)
        #expect(rowSummary(model) == "3 / 3 rows · 3 columns")
        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: a)
        await settled(model)
        #expect(rowSummary(model) == "1 / 3 rows · 3 columns")
        model.clearFilters()
        await settled(model)
        #expect(rowSummary(model) == "3 rows · 3 columns")
    }

    @Test func sortChangesKeepTheRowSetAndItsAnalyses() async throws {
        let model = try await model()
        defer { model.stop() }
        let a = model.id("a")
        let rows = model.displayed.rowSet
        model.setSortDirection(column: a, descending: true)
        await settled(model)
        #expect(model.displayed.rowSet === rows)
        #expect(Array(model.displayed.rowNumbers(0..<3)) == [2, 0, 1])
    }

    @Test func replacingSortColumnKeepsPriorityDirectionAndFilters() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        let filter = Filter(condition: .range(min: "1", max: nil), includeNulls: true)
        model.setFilter(filter, column: a)
        model.setSortDirection(column: a, descending: true)
        model.setSortDirection(column: b, descending: false)
        model.replaceSortColumn(a, with: c)
        await settled(model)
        #expect(
            model.appliedQuery.sorts == [
                ColumnSort(column: c, descending: true), ColumnSort(column: b, descending: false),
            ])
        #expect(model.appliedQuery.filter(for: a) == filter)
        model.replaceSortColumn(c, with: b)
        #expect(model.draft.sorts.map(\.column) == [c, b], "A column already sorted cannot replace another")
    }

    @Test func clearingTheQueryKeepsColumnArrangementFindAndInspection() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        model.setColumnsVisible([c], visible: false)
        model.reorderColumns([b, a])
        model.find.query = "1"
        model.inspect(a)
        model.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: a)
        model.setSortDirection(column: b, descending: true)
        await settled(model)
        model.clearFilters()
        model.clearSorts()
        await settled(model)
        #expect(model.displayed.count == 3 && model.appliedQuery == TableQuery())
        #expect(model.visibleColumns == [b, a])
        #expect(model.find.query == "1" && model.inspectedColumn == a)
    }

    @Test func hidingAColumnAppliesAtOnceAndClearsItsConditions() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b) = (model.id("a"), model.id("b"))
        model.setFilter(exact("1"), column: a)
        model.setSortDirection(column: a, descending: true)
        await settled(model)
        #expect(model.displayed.count == 2)

        model.setColumnsVisible([a], visible: false)
        #expect(!model.visibleColumns.contains(a), "Visibility is presentation and applies immediately")
        #expect(model.draft == TableQuery())
        await settled(model)
        #expect(model.displayed.count == 3 && model.appliedQuery == TableQuery())

        model.find.query = "1"
        await eventually { model.find.hasMatches != nil }
        #expect(model.find.currentMatch?.column == b, "Find searches only visible columns")
        model.setColumnsVisible([a], visible: true)
        #expect(model.visibleColumns.last == a)
        #expect(model.draft.filter(for: a) == nil && model.draft.sort(for: a) == nil)
    }

    @Test func showAllRestoresHiddenColumnsWithoutTheirConditions() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        model.reorderColumns([c, b, a])
        model.setSortDirection(column: a, descending: true)
        model.setColumnsVisible([a], visible: false)
        await settled(model)
        model.showAllColumns()
        #expect(model.visibleColumns == [c, b, a])
        #expect(model.appliedQuery.sort(for: a) == nil && model.appliedQuery.filter(for: a) == nil)
    }

    @Test func removingOneFilterKeepsOtherFiltersSortingAndFind() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        model.find.query = "x"
        model.setFilter(exact("1"), column: a)
        model.setFilter(exact("x"), column: c)
        model.setSortDirection(column: b, descending: true)
        await settled(model)
        model.reorderColumns([c, a, b])
        model.setFilter(nil, column: c)
        await settled(model)
        #expect(model.filteredColumns == [a] && model.displayed.count == 2)
        #expect(model.appliedQuery.sorts.map(\.column) == [b] && model.find.query == "x")
        model.clearFilters()
        await settled(model)
        #expect(model.displayed.count == 3 && model.filteredColumns.isEmpty)
        #expect(model.appliedQuery.sorts.map(\.column) == [b])
    }

    @Test func sortingCyclesAndPrioritiesAreIndependentOfDisplayOrder() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        model.cycleSort(column: a, multiple: false)
        model.cycleSort(column: b, multiple: true)
        #expect(model.draft.sorts.map(\.column) == [a, b])
        model.reorderColumns([c, b, a])
        #expect(model.draft.sorts.map(\.column) == [a, b])
        model.cycleSort(column: a, multiple: true)
        #expect(model.draft.sort(for: a)?.descending == true)
        model.cycleSort(column: a, multiple: true)
        #expect(model.draft.sorts.map(\.column) == [b])
        model.cycleSort(column: c, multiple: false)
        #expect(model.draft.sorts.map(\.column) == [c])
        model.reorderSorts([c])
        model.clearSorts()
        await settled(model)
        #expect(model.visibleColumns == [c, b, a])
        #expect(Array(model.displayed.rowNumbers(0..<10)) == [0, 1, 2])
    }

    @Test func hidingSelectedInspectedAndChartColumnsFallsBackToVisibleOnes() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        #expect(model.chartX == nil && model.chartY == a)
        model.select(a)
        model.inspect(a)
        model.chartX = a

        model.setColumnsVisible([a], visible: false)
        #expect(model.selectedColumn == b && model.inspectedColumn == b)
        #expect(model.chartX == nil && model.chartY == b)

        // Without a plottable column, the Y axis is empty while selection stays on c.
        model.setColumnsVisible([b], visible: false)
        #expect(model.selectedColumn == c && model.inspectedColumn == c && model.chartY == nil)

        model.setColumnsVisible([c], visible: false)
        #expect(model.selectedColumn == nil && model.inspectedColumn == nil)
        #expect(model.chartX == nil && model.chartY == nil)

        model.setColumnsVisible([c], visible: true)
        #expect(model.selectedColumn == c && model.inspectedColumn == c && model.chartY == nil)
        model.setColumnsVisible([b], visible: true)
        #expect(model.chartX == nil && model.chartY == b)

        model.setColumnsVisible([c, b], visible: false)
        model.inspect(a)
        #expect(model.inspectedColumn == nil, "Hidden columns cannot be inspected")
        model.showAllColumns()
        #expect(model.selectedColumn == a && model.inspectedColumn == a && model.chartY == a)
    }

    @Test func filtersKeepSchemaOrderSoEqualConditionsNeedNoQuery() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, c) = (model.id("a"), model.id("c"))
        model.setFilter(exact("x"), column: c)
        model.setFilter(exact("1"), column: a)
        await settled(model)
        let rows = model.displayed
        #expect(model.appliedQuery.filters.map(\.column) == [a, c])

        model.setFilter(nil, column: a)
        model.setFilter(exact("1"), column: a)
        #expect(model.draft.filters.map(\.column) == [a, c])
        #expect(!model.isUpdating && model.displayed === rows)
    }

    @Test func revertingAPendingFilterCancelsItsQuery() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, c) = (model.id("a"), model.id("c"))
        model.setFilter(exact("1"), column: a)
        model.setColumnsVisible([c], visible: false)
        model.setFilter(nil, column: a)
        #expect(!model.isUpdating)
        #expect(model.visibleColumns == [a, model.id("b")])
    }

    @Test func failedQueriesKeepTheDisplayedRowsAndRestoreTheDraft() async throws {
        let alerts = Alerts()
        let model = try await model(alerts: alerts)
        defer { model.stop() }
        let a = model.id("a")
        model.setFilter(exact("1"), column: a)
        await settled(model)
        let rows = model.displayed
        model.setFilter(Filter(condition: .range(min: "invalid", max: nil), includeNulls: false), column: a)
        await settled(model)
        #expect(alerts.current?.message.contains("invalid") == true)
        #expect(model.displayed === rows && model.draft == rows.query)
    }

    @Test func bulkVisibilityClearsOnlyHiddenConditionsAndKeepsRemainingOrder() async throws {
        let model = try await model()
        defer { model.stop() }
        let (a, b, c) = (model.id("a"), model.id("b"), model.id("c"))
        model.reorderColumns([c, b, a])
        model.setFilter(exact("x"), column: c)
        model.setSortDirection(column: b, descending: true)
        model.setSortDirection(column: a, descending: false)
        await settled(model)
        model.setColumnsVisible([c, b], visible: false)
        await settled(model)
        #expect(model.visibleColumns == [a] && model.displayed.count == 3)
        #expect(model.appliedQuery.sorts == [ColumnSort(column: a, descending: false)])
        model.setColumnsVisible([c, b], visible: true)
        #expect(model.visibleColumns == [a, b, c])
    }

    @Test func windowsSharingASessionKeepIndependentQueries() async throws {
        let session = try await TestTables.session(Self.csv)
        let alerts = Alerts()
        let first = TableModel(url: URL(fileURLWithPath: "/a.csv"), session: session, alerts: alerts)
        let second = TableModel(url: URL(fileURLWithPath: "/a.csv"), session: session, alerts: alerts)
        defer {
            first.stop()
            second.stop()
        }
        first.setFilter(Filter(condition: .range(min: "2", max: nil), includeNulls: false), column: first.id("a"))
        second.setFilter(exact("y"), column: second.id("c"))
        await settled(first)
        await settled(second)
        #expect(Array(first.displayed.rowNumbers(0..<3)) == [2])
        #expect(Array(second.displayed.rowNumbers(0..<3)) == [1])
        first.stop()
        #expect(try await second.displayed.rowSet.nullCount(second.id("a")) == 0)
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
        guard case .exact(let values, let exclude) = filter?.condition else {
            Issue.record("Expected exact selection")
            return
        }
        #expect(exclude && values.count == 1)
        let onlyNull = NullMode.only.applying(to: draft.condition)
        #expect(onlyNull == Filter(condition: .exact(values: [], exclude: false), includeNulls: true))
        draft.all(true)
        #expect(NullMode.include.applying(to: draft.condition) == nil)
    }

    @Test func valueSelectionTreatsMissingAsAValueAndResetClearsEveryDraft() async throws {
        let model = try await TestTables.model("value,number\nalpha,1\n,2\nMissing (null),3\nbeta,4\n")
        defer { model.stop() }
        let value = model.id("value")
        var draft = ColumnFilterDraft(nil)
        draft.selectAllValues(false)
        model.setFilter(draft.filter(mode: .exact), column: value)
        await settled(model)
        #expect(model.displayed.count == 0)

        // Reopening a zero-row result can recover through the column's own options.
        let options = try await model.displayed.rowSet.filterOptions(for: value)
        #expect(options.values?.count == 3 && options.nullCount == 1)
        var reopened = ColumnFilterDraft(model.appliedQuery.filter(for: value))
        reopened.enterValues()
        reopened.selectAllValues(true)
        #expect(reopened.filter(mode: .exact) == nil)

        draft.includesMissing = true
        model.setFilter(draft.filter(mode: .exact), column: value)
        await settled(model)
        #expect(Array(model.displayed.rowNumbers(0..<10)) == [1])

        draft = ColumnFilterDraft(draft.filter(mode: .exact))
        draft.enterValues()
        draft.includesMissing = true
        draft.setValue(LiteralTextValue(value: "Missing (null)"), selected: true)
        model.setFilter(draft.filter(mode: .exact), column: value)
        await settled(model)
        #expect(Array(model.displayed.rowNumbers(0..<10)) == [1, 2])

        draft.selectAllValues(true)
        #expect(draft.filter(mode: .exact) == nil)
        draft.text.text = "alpha"
        draft.range.minimum = "3"
        draft.range.lowerEdited = true
        draft.nullMode = .only
        draft.reset()
        #expect(draft.filter(mode: .exact) == nil && draft.filter(mode: .text) == nil)
        #expect(draft.filter(mode: .range) == nil)
        #expect(model.displayed.count == 2, "Reset edits the draft, not the displayed rows")
    }
}
