import AppKit
import Testing

struct ColumnWidthTests {
    @Test func manualResizeWinsOverQueuedSamplesAndNewFitWinsOverOldFit() {
        var widths = ColumnWidths()
        let initial = widths.begin(40)
        widths.manual(40)
        let acceptedInitial = widths.accept(initial)
        #expect(!acceptedInitial)
        #expect(!widths.needsAutomatic(40))
        widths.fontChanged()
        #expect(!widths.needsAutomatic(40))
        let old = widths.begin(40)
        let new = widths.begin(40)
        let acceptedOld = widths.accept(old)
        let acceptedNew = widths.accept(new)
        #expect(!acceptedOld)
        #expect(acceptedNew)
        widths.fontChanged()
        #expect(widths.needsAutomatic(40))
    }

    @Test func cancelledSampleCanRetryWithoutUndoingNewerManualWidth() {
        var widths = ColumnWidths()
        let old = widths.begin(0)
        widths.abandon(old)
        #expect(widths.needsAutomatic(0))
        let next = widths.begin(0)
        widths.manual(0)
        widths.abandon(next)
        #expect(!widths.needsAutomatic(0))
    }

    @Test func measurementIsBoundedForLargeUnicodeCellsAndCancelledBeforeRead() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-width-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try ("value\n" + "a" + String(repeating: "\u{301}", count: 100_000) + "\n").write(
            to: url, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: url.path).view()
        let input = ColumnWidthMeasurement.Input(column: 0, name: "value", type: "Text", text: true)
        let request = ColumnWidthMeasurement.Request()
        let result = try ColumnWidthMeasurement.measure(
            view: view, columns: [input], start: 0, fontSize: 12, maximum: 420, cancellation: request)
        #expect(try #require(result[0]) <= 420)
        request.cancel()
        #expect(throws: CancellationError.self) {
            try ColumnWidthMeasurement.measure(
                view: view, columns: [input], start: 0, fontSize: 12, maximum: 420, cancellation: request)
        }
    }

    @Test func fitUsesAnAvailableSampleInsteadOfRereadingTheView() throws {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-fit-cache-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try "value\nx\n".write(to: url, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: url.path).view()
        // An invalid source index would fail if the measurement ignored its sample.
        let cached = ColumnWidthMeasurement.Input(
            column: 99, name: "value", type: "Text", text: true, sample: ["cached value"])
        let widths = try ColumnWidthMeasurement.measure(
            view: view, columns: [cached], start: 0, fontSize: 12, maximum: 600, cancellation: .init())
        #expect(widths[99] != nil)
    }
}
