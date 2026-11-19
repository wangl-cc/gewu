import AppKit
import Testing

struct ColumnWidthTests {
    @Test func manualResizeWinsOverQueuedSamplesAndNewFitWinsOverOldFit() {
        var widths = ColumnWidths()
        let col = ColumnId(raw: 40)
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
        #expect(!acceptedOld)
        #expect(acceptedNew)
        widths.fontChanged()
        #expect(widths.needsAutomatic(col))
    }

    @Test func cancelledSampleCanRetryWithoutUndoingNewerManualWidth() {
        var widths = ColumnWidths()
        let col = ColumnId(raw: 0)
        let old = widths.begin(col)
        widths.abandon(old)
        #expect(widths.needsAutomatic(col))
        let next = widths.begin(col)
        widths.manual(col)
        widths.abandon(next)
        #expect(!widths.needsAutomatic(col))
    }

    @Test func measurementIsBoundedForLargeUnicodeCellsAndCancelledBeforeRead() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-width-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try ("value\n" + "a" + String(repeating: "\u{301}", count: 100_000) + "\n").write(
            to: url, atomically: true, encoding: .utf8)
        let ordered = try GewuDataset.open(path: url.path).unfiltered().unsorted()
        let col = ColumnId(raw: 0)
        let input = ColumnWidthMeasurement.Input(column: col, name: "value", type: "Text", text: true)
        let request = ColumnWidthMeasurement.Request()
        let result = try ColumnWidthMeasurement.measure(
            ordered: ordered, columns: [input], start: 0, fontSize: 12, maximum: 420, cancellation: request)
        #expect(try #require(result[col]) <= 420)
        request.cancel()
        #expect(throws: CancellationError.self) {
            try ColumnWidthMeasurement.measure(
                ordered: ordered, columns: [input], start: 0, fontSize: 12, maximum: 420, cancellation: request)
        }
    }

    @Test func fitUsesAnAvailableSampleInsteadOfRereadingTheView() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-fit-cache-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try "value\nx\n".write(to: url, atomically: true, encoding: .utf8)
        let ordered = try GewuDataset.open(path: url.path).unfiltered().unsorted()
        // An invalid source index would fail if the measurement ignored its sample.
        let bogus = ColumnId(raw: 99)
        let cached = ColumnWidthMeasurement.Input(
            column: bogus, name: "value", type: "Text", text: true,
            sample: TableExecutionTests.text(["cached value"]))
        let widths = try ColumnWidthMeasurement.measure(
            ordered: ordered, columns: [cached], start: 0, fontSize: 12, maximum: 600, cancellation: .init())
        #expect(widths[bogus] != nil)
    }
}
