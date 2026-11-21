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
            guard let error = error as? GewuKitError, case .io(.canonicalize) = error.kind else { return false }
            // The file system's own message is the only account of what happened.
            return error.diagnostic.contains(url.lastPathComponent)
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

    @Test func canonicallyEquivalentValuesAreDifferentQueries() async throws {
        let file = try Fixture("name\n\u{E9}\ne\u{301}\n")
        let session = try await file.open()
        let name = session.source.id("name")
        let composed = try await session.apply(
            TableQuery(filters: [ColumnFilter(column: name, filter: exact("\u{E9}"))]), after: session.initialRows)
        #expect(Array(composed.rowNumbers(0..<2)) == [0])
        let decomposed = try await session.apply(
            TableQuery(filters: [ColumnFilter(column: name, filter: exact("e\u{301}"))]), after: composed)
        #expect(decomposed.rowSet !== composed.rowSet)
        #expect(Array(decomposed.rowNumbers(0..<2)) == [1], "The engine keeps the spellings apart, and so does the query")
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
            guard let error = error as? GewuKitError else { return false }
            return error.kind
                == .filter(.invalidValue(value: "not a number", expectedType: "Int64", reason: .invalidSyntax))
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
        await #expect {
            try await TableSession.open(url, options: OpenOptions(sheet: "absent"))
        } throws: { error in
            (error as? GewuKitError)?.kind == .file(.sheetNotFound("absent"))
        }
    }
}
