import Foundation
import GewuKit
import Testing

@Suite struct TableSessionTests {
    @Test func openingDescribesTheFileAndShowsEveryRow() async throws {
        let file = try Fixture("city,score\nbj,10\nsh,20\nbj,30\n")
        let session = try await file.open()
        let info = session.source.info
        #expect(info.format == .csv && info.rowCount == 3)
        #expect(info.columns.map(\.name) == ["city", "score"])
        #expect(session.initialRows.count == 3 && session.initialRows.query == TableQuery())
        #expect(Array(session.initialRows.rowNumbers(0..<10)) == [0, 1, 2])
    }

    @Test func openingAMissingFileReportsFileAccess() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("absent-\(UUID()).csv")
        await #expect {
            try await TableSession.open(url)
        } throws: { error in
            if case .fileAccess = error as? GewuKitError { true } else { false }
        }
    }

    @Test func applyReusesWhatTheQueryDidNotChange() async throws {
        let file = try Fixture("city,score\nbj,10\nsh,20\nbj,30\n")
        let session = try await file.open()
        let city = session.source.id("city")
        let score = session.source.id("score")
        let initial = session.initialRows

        #expect(try await session.apply(TableQuery(), after: initial) === initial)

        let sorted = try await session.apply(
            TableQuery(sorts: [ColumnSort(column: score, descending: true)]), after: initial)
        #expect(sorted.rowSet === initial.rowSet, "A sort change keeps the row set and its analyses")
        #expect(Array(sorted.rowNumbers(0..<3)) == [2, 1, 0])

        let filtered = try await session.apply(
            TableQuery(filters: [ColumnFilter(column: city, filter: exact("bj"))], sorts: sorted.sorts), after: sorted)
        #expect(filtered.rowSet !== sorted.rowSet && filtered.count == 2)
        #expect(Array(filtered.rowNumbers(0..<3)) == [2, 0], "The sort applies to the new rows")
        #expect(filtered.query.filters.count == 1)
    }

    @Test func invalidFilterValuesAreUserErrors() async throws {
        let file = try Fixture("score\n1\n")
        let session = try await file.open()
        let query = TableQuery(filters: [
            ColumnFilter(
                column: session.source.id("score"),
                filter: .condition(.range(min: "not a number", max: nil), includeNulls: false))
        ])
        await #expect {
            try await session.apply(query, after: session.initialRows)
        } throws: { error in
            guard case .invalidFilter(let message) = error as? GewuKitError else { return false }
            return message.contains("not a number") && error.localizedDescription == message
        }
    }

    @Test func workbooksListAndOpenWorksheets() async throws {
        let csv = try Fixture("label\ncat\n")
        let session = try await csv.open()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("book-\(UUID()).xlsx")
        defer { try? FileManager.default.removeItem(at: url) }
        try await session.initialRows.export(session.source.columns.map(\.id), format: .xlsx).write(to: url)

        let sheets = try await TableSession.worksheetNames(url)
        #expect(sheets.count == 1)
        let book = try await TableSession.open(url, options: OpenOptions(sheet: sheets[0]))
        #expect(book.source.info.worksheet == sheets[0])
        await #expect(throws: GewuKitError.self) {
            try await TableSession.open(url, options: OpenOptions(sheet: "absent"))
        }
    }
}
