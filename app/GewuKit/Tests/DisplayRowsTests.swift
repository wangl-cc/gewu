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

    @Test func findStepsAndHighlightsInDisplayOrder() async throws {
        let file = try Fixture("value,other\nhit,12\nmiss,120\nhit,12\n")
        let session = try await file.open()
        let value = session.source.id("value")
        let other = session.source.id("other")
        let find = try session.initialRows.find("12", in: [value, other])

        // Numbers match whole values, so 120 is not a match for 12.
        let first = CellMatch(displayRow: 0, column: other)
        let last = CellMatch(displayRow: 2, column: other)
        #expect(try await find.step(1, from: nil) == first)
        #expect(try await find.step(-1, from: nil) == last)
        #expect(try await find.step(1, from: last) == first, "Wraps at the end")
        #expect(try await find.step(3, from: first) == last)
        #expect(try await session.initialRows.find("absent", in: [value]).step(1, from: nil) == nil)

        let flags = try await find.highlight([other, value], in: 0..<64)
        #expect(flags.rows == 0..<3)
        #expect((0..<3).map { flags.isMatch(row: $0, column: other) } == [true, false, true])
        #expect(!flags.isMatch(row: 0, column: value))
    }

    @Test func emptyFindQueriesAreRejected() async throws {
        let file = try Fixture("value\nx\n")
        let session = try await file.open()
        #expect(throws: GewuKitError.self) {
            try session.initialRows.find("", in: [session.source.id("value")])
        }
    }

    @Test func sourceRowsAreReadInPlaceForEveryOrder() async throws {
        let file = try Fixture("n\n3\n1\n2\n")
        let session = try await file.open()
        #expect(Array(session.initialRows.sourceRows) == [0, 1, 2], "File order needs no mapping")
        let sorted = try await session.apply(
            TableQuery(sorts: [ColumnSort(column: session.source.id("n"), descending: false)]),
            after: session.initialRows)
        #expect(Array(sorted.sourceRows) == [1, 2, 0])
        #expect(Array(sorted.rowNumbers(2..<10)) == [0], "Clipped to the available rows")
    }

    @Test func tabSeparatedTextQuotesFieldsAndRespectsItsLimit() async throws {
        let file = try Fixture("a,b\n\"x\ty\",\"say \"\"hi\"\"\"\nplain,\n")
        let session = try await file.open()
        let columns = session.source.columns.map(\.id)
        let rows = session.initialRows
        let data = try await rows.tabSeparatedText(
            rows: [0..<2], columns: columns, includeHeader: true, byteLimit: 1024)
        // The same encoding as a TSV export.
        #expect(
            String(decoding: data, as: UTF8.self) == "a\tb\r\n\"x\ty\"\t\"say \"\"hi\"\"\"\r\nplain\t\r\n")
        let headless = try await rows.tabSeparatedText(
            rows: [1..<2], columns: columns, includeHeader: false, byteLimit: 1024)
        #expect(String(decoding: headless, as: UTF8.self) == "plain\t\r\n")
        await #expect {
            try await rows.tabSeparatedText(rows: [0..<2], columns: columns, includeHeader: true, byteLimit: 10)
        } throws: { error in
            (error as? GewuKitError)?.kind == .export(.sizeLimit(10))
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
        let samples = ProgressLog()
        try await job.write(to: destination, progressInterval: .milliseconds(5)) { samples.reports.append($0) }
        #expect(await samples.reports.last == ExportProgress(rowsWritten: 3, totalRows: 3))
        #expect(try String(contentsOf: destination, encoding: .utf8) == "id\r\n3\r\n2\r\n1\r\n")
    }
}

@MainActor
private final class ProgressLog: Sendable {
    var reports: [ExportProgress] = []
}
