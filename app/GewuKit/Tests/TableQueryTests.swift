import GewuKit
import Testing

@Suite struct TableQueryTests {
    private func columns() async throws -> (a: ColumnID, b: ColumnID) {
        let file = try Fixture("a,b\n1,2\n")
        let source = try await file.open().source
        return (source.id("a"), source.id("b"))
    }

    @Test func literalsCompareByBytesAsTheEngineDoes() async throws {
        let (a, _) = try await columns()
        let composed = "\u{E9}"
        let decomposed = "e\u{301}"
        #expect(composed == decomposed, "String equality merges canonically equivalent spellings")
        let values = { (value: String) in FilterCondition.exact(values: [value], exclude: false) }
        #expect(values(composed) != values(decomposed))
        #expect(values(composed) == values("\u{E9}"))
        #expect(Set([values(composed), values(decomposed)]).count == 2)
        #expect(
            FilterCondition.text(text: composed, mode: .contains, caseSensitive: true)
                != .text(text: decomposed, mode: .contains, caseSensitive: true))
        #expect(FilterCondition.range(min: composed, max: nil) != .range(min: decomposed, max: nil))
        #expect(FilterCondition.range(min: nil, max: "1") == .range(min: nil, max: "1"))

        let query = { (value: String) in
            TableQuery(filters: [ColumnFilter(column: a, filter: .condition(values(value), includeNulls: false))])
        }
        #expect(query(composed) != query(decomposed), "A query of either spelling runs")
    }

    @Test func filtersKeepOneOrderWhateverOrderTheyWereSetIn() async throws {
        let (a, b) = try await columns()
        var first = TableQuery()
        first.setFilter(.notNull, for: b)
        first.setFilter(.onlyNulls, for: a)
        var second = TableQuery()
        second.setFilter(.onlyNulls, for: a)
        second.setFilter(.notNull, for: b)
        #expect(first == second)
        #expect(first.filters.map(\.column) == [a, b])
        #expect(
            TableQuery(filters: [ColumnFilter(column: b, filter: .notNull), ColumnFilter(column: a, filter: .onlyNulls)])
                == first)

        first.setFilter(.notNull, for: a)
        #expect(first.filters.map(\.column) == [a, b] && first.filter(for: a) == .notNull)
        first.setFilter(nil, for: a)
        first.removeFilters { $0.column == b }
        #expect(first == TableQuery())
    }
}
