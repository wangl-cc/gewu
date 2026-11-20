import Foundation
import GewuKit
import Testing

@Suite struct DisplayRowsTests {
    @Test func columnTextDecodesCellsInPlace() async throws {
        // CSV reads empty fields as missing; Core's segment tests cover empty text.
        let file = try Fixture("text,n\né,1\n,2\n界🦀,3\n")
        let session = try await file.open()
        let rows = session.initialRows
        let text = try await rows.text(of: session.source.id("text"), rows: 1..<64)
        #expect(text.rows == 1..<3, "Clipped to the available rows")
        #expect(text.cell(0) == nil, "Rows outside the block have no text")
        #expect([text.cell(1), text.cell(2)] == [nil, "界🦀"])
        #expect(text.withUTF8(2) { $0.count } == "界🦀".utf8.count)
        #expect(text.byteCount > 0)
    }

    @Test func textOutlivesTheRowsItWasReadFrom() async throws {
        let file = try Fixture("n\n1\n2\n")
        var session: TableSession? = try await file.open()
        let column = session!.source.id("n")
        let text = try await session!.initialRows.text(of: column, rows: 0..<2)
        session = nil
        #expect(text.cell(1) == "2")
    }

    @Test func searchPagesAndHighlightsInDisplayOrder() async throws {
        let file = try Fixture("value,other\nhit,12\nmiss,120\nhit,12\n")
        let session = try await file.open()
        let value = session.source.id("value")
        let other = session.source.id("other")
        let search = try session.initialRows.search("12", in: [value, other])

        // Numbers match whole values, so 120 is not a match for 12.
        let page = try await search.matches(after: nil, limit: 2)
        #expect(page == [CellMatch(displayRow: 0, column: other), CellMatch(displayRow: 2, column: other)])
        let last = try await search.matches(before: nil, limit: 1)
        #expect(last == [CellMatch(displayRow: 2, column: other)])
        #expect(try await search.matches(after: last[0], limit: 4).isEmpty, "Empty at the end")

        let flags = try await search.highlights([other, value], rows: 0..<64)
        #expect(flags.rows == 0..<3)
        #expect((0..<3).map { flags.isMatch(row: $0, column: other) } == [true, false, true])
        #expect(!flags.isMatch(row: 0, column: value))
    }

    @Test func invalidSearchesAreUserErrors() async throws {
        let file = try Fixture("value\nx\n")
        let session = try await file.open()
        #expect(throws: GewuKitError.self) {
            try session.initialRows.search("", in: [session.source.id("value")])
        }
    }

    @Test func tabSeparatedTextQuotesFieldsAndRespectsItsLimit() async throws {
        let file = try Fixture("a,b\n\"x\ty\",\"say \"\"hi\"\"\"\nplain,\n")
        let session = try await file.open()
        let columns = session.source.columns.map(\.id)
        let rows = session.initialRows
        let data = try await rows.tabSeparatedText(
            rows: [0..<2], columns: columns, includeHeader: true, byteLimit: 1024)
        #expect(String(decoding: data, as: UTF8.self) == "a\tb\n\"x\ty\"\t\"say \"\"hi\"\"\"\nplain\t")
        await #expect(throws: TextLimitExceeded.self) {
            try await rows.tabSeparatedText(rows: [0..<2], columns: columns, includeHeader: true, byteLimit: 10)
        }
    }

    @Test func exportWritesTheCapturedRowsAndFinishesProgress() async throws {
        let file = try Fixture("id\n1\n2\n3\n")
        let session = try await file.open()
        let id = session.source.id("id")
        let sorted = try await session.apply(
            TableQuery(sorts: [ColumnSort(column: id, descending: true)]), after: session.initialRows)
        let destination = FileManager.default.temporaryDirectory.appendingPathComponent("export-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: destination) }

        let job = try sorted.export([id], format: .csv)
        async let samples = Array(collecting: job.progress(every: .milliseconds(5)))
        try await job.write(to: destination)
        #expect(await samples.last == ExportProgress(rowsWritten: 3, totalRows: 3))
        #expect(try String(contentsOf: destination, encoding: .utf8) == "id\r\n3\r\n2\r\n1\r\n")
        #expect(try sorted.nestedColumns([id]).isEmpty)
    }
}

extension Array {
    fileprivate init<S: AsyncSequence>(collecting sequence: S) async rethrows where S.Element == Element {
        self = []
        for try await element in sequence { append(element) }
    }
}
