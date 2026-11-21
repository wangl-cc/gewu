import Foundation
import GewuKit
import Testing

struct HistogramScaleTests {
    @Test
    @MainActor
    static func changingBinningRefreshesTheInspectorWithoutFiltering() async throws {
        let session = try await TestTables.session("value\n-1\n0\n1\n10\n100\n")
        let rows = session.initialRows.rowSet
        let model = InspectorModel()
        for scale in [AxisScale.linear, .log10, .linear] {
            await model.load(
                .init(
                    rows: rows, column: session.source.id("value"), expanded: [.distribution], bins: 2, scale: scale,
                    attempt: 0))
            guard let histogram = model.distribution else {
                Issue.record("Missing distribution: \(model.errors)")
                return
            }
            #expect(histogram.scale == scale)
            #expect(histogram.nonPositiveCount == (scale == .log10 ? 2 : 0))
            #expect(histogram.counts.reduce(0, +) == (scale == .log10 ? 3 : 5))
        }
        #expect(rows.count == 5)
    }

    @Test
    static func logFrequencyKeepsEmptyBinsEmptyAndEqualRatiosEquallySpaced() {
        #expect(HistogramFrequency.fraction(0, maximum: 100, scale: .log10) == 0)
        let low = HistogramFrequency.fraction(1, maximum: 100, scale: .log10)
        let middle = HistogramFrequency.fraction(10, maximum: 100, scale: .log10)
        let high = HistogramFrequency.fraction(100, maximum: 100, scale: .log10)
        #expect(low > 0 && high == 1)
        #expect(abs((middle - low) - (high - middle)) < 1e-12)
        #expect(HistogramFrequency.fraction(1, maximum: 1, scale: .log10) == 1)
    }
}
