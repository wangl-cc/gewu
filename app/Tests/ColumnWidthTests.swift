import AppKit
import GewuKit
import Testing

struct ColumnWidthTests {
    private static func columns(_ count: Int) async throws -> [ColumnID] {
        let header = (0..<count).map { "c\($0)" }.joined(separator: ",")
        return try await TestTables.session(header + "\n" + (0..<count).map(String.init).joined(separator: ",") + "\n")
            .source.columns.map(\.id)
    }

    @Test func manualResizeWinsOverQueuedSamplesAndNewFitWinsOverOldFit() async throws {
        var widths = ColumnWidths()
        let col = try await Self.columns(1)[0]
        let initial = widths.begin(col)
        widths.manual(col)
        let acceptedInitial = widths.accept(initial)
        #expect(!acceptedInitial)
        #expect(!widths.needsAutomatic(col))
        widths.fontChanged()
        #expect(!widths.needsAutomatic(col))
        let old = widths.begin(col)
        let new = widths.begin(col)
        let acceptedOld = widths.accept(old)
        let acceptedNew = widths.accept(new)
        #expect(!acceptedOld && acceptedNew)
        widths.fontChanged()
        #expect(widths.needsAutomatic(col))
    }

    @Test func cancelledSampleCanRetryWithoutUndoingNewerManualWidth() async throws {
        var widths = ColumnWidths()
        let col = try await Self.columns(1)[0]
        let old = widths.begin(col)
        widths.abandon(old)
        #expect(widths.needsAutomatic(col))
        let next = widths.begin(col)
        widths.manual(col)
        widths.abandon(next)
        #expect(!widths.needsAutomatic(col))
    }

    @Test func measurementIsBoundedForLargeUnicodeCellsAndStopsWhenCancelled() async throws {
        let session = try await TestTables.session("value\na" + String(repeating: "\u{301}", count: 100_000) + "\n")
        let col = session.source.id("value")
        let input = ColumnWidthMeasurement.Input(column: col, name: "value", type: "Text", text: true)
        let result = try await ColumnWidthMeasurement.measure(
            rows: session.initialRows, columns: [input], start: 0, fontSize: 12, maximum: 420)
        #expect(try #require(result[col]) <= 420)
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await ColumnWidthMeasurement.measure(
                rows: session.initialRows, columns: [input], start: 0, fontSize: 12, maximum: 420)
        }
        await #expect(throws: CancellationError.self) { try await cancelled.value }
    }

    @Test func fitUsesTheSameRowsWithAndWithoutLoadedText() async throws {
        let values = [String(repeating: "W", count: 80)] + Array(repeating: "x", count: 63)
        let session = try await TestTables.session("value\n" + values.joined(separator: "\n"))
        let column = session.source.id("value")
        let loaded = try await session.initialRows.text(of: column, rows: 0..<64)
        var input = ColumnWidthMeasurement.Input(column: column, name: "value", type: "Text", text: true)
        let cold = try await ColumnWidthMeasurement.measure(
            rows: session.initialRows, columns: [input], start: 1, fontSize: 12, maximum: 800)
        input.sample = loaded
        let hot = try await ColumnWidthMeasurement.measure(
            rows: session.initialRows, columns: [input], start: 1, fontSize: 12, maximum: 800)
        #expect(hot == cold, "Row 0's wide value is outside the sampled rows either way")
    }
}
