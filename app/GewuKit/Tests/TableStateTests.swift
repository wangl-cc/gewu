import GewuKit
import Testing

@Suite struct TableStateTests {
    /// `a` and `b` are numbers, which can be plotted; `c` is text.
    private func state() async throws -> (TableState, a: ColumnID, b: ColumnID, c: ColumnID) {
        let file = try Fixture("a,b,c\n1,2,x\n1,1,y\n2,0,z\n")
        let source = try await file.open().source
        return (TableState(columns: source.columns), source.id("a"), source.id("b"), source.id("c"))
    }

    @Test func sortsCycleAndKeepPriorityWhateverTheColumnOrder() async throws {
        var (state, a, b, c) = try await state()
        state.cycleSort(a, multiple: false)
        state.cycleSort(b, multiple: true)
        #expect(state.query.sorts.map(\.column) == [a, b])
        state.reorderColumns([c, b, a])
        #expect(state.query.sorts.map(\.column) == [a, b], "Column order is not sort priority")
        state.cycleSort(a, multiple: true)
        #expect(state.query.sort(for: a)?.descending == true)
        state.cycleSort(a, multiple: true)
        #expect(state.query.sorts.map(\.column) == [b])
        state.cycleSort(c, multiple: false)
        #expect(state.query.sorts == [ColumnSort(column: c, descending: false)], "A plain click sorts by one column")
        state.replaceSortColumn(c, with: a)
        #expect(state.query.sorts == [ColumnSort(column: a, descending: false)])
        #expect(state.visibleColumns == [c, b, a])
    }

    @Test func filtersKeepSchemaOrder() async throws {
        var (state, a, _, c) = try await state()
        let x = Filter.condition(.exact(values: ["x"], exclude: false), includeNulls: false)
        state.setFilter(x, for: c)
        state.setFilter(.notNull, for: a)
        #expect(state.query.filters.map(\.column) == [a, c], "Equal conditions give equal queries")
        state.setFilter(nil, for: c)
        #expect(state.query.filters == [ColumnFilter(column: a, filter: .notNull)])
    }

    @Test func hidingAColumnClearsItsConditionsAndMovesChoicesToVisibleColumns() async throws {
        var (state, a, b, c) = try await state()
        #expect(state.selectedColumn == a && state.chartX == nil && state.chartY == a)
        state.setChartX(a)
        state.setFilter(.notNull, for: a)
        state.cycleSort(a, multiple: false)

        state.setColumnsVisible([a], visible: false)
        #expect(state.query == TableQuery())
        #expect(state.selectedColumn == b && state.inspectedColumn == b)
        #expect(state.chartX == nil && state.chartY == b)

        // Without a plottable column, the y axis is empty while the selection moves to c.
        state.setColumnsVisible([b], visible: false)
        #expect(state.selectedColumn == c && state.inspectedColumn == c && state.chartY == nil)
        state.setChartY(c)
        #expect(state.chartY == nil, "Text cannot be plotted")

        state.setColumnsVisible([c], visible: false)
        #expect(state.selectedColumn == nil && state.inspectedColumn == nil)
        state.inspect(a)
        #expect(state.inspectedColumn == nil, "Hidden columns cannot be inspected")

        state.showAllColumns()
        #expect(state.visibleColumns == [a, b, c], "Shown columns return in file order")
        #expect(state.selectedColumn == a && state.inspectedColumn == a && state.chartY == a)
    }
}
