import Foundation
import Testing

struct HistogramScaleTests {
    @Test
    @MainActor
    static func changingBinningRefreshesTheInspectorWithoutFiltering() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).csv")
        try "value\n-1\n0\n1\n10\n100\n".write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        let document = try await Task.detached { try GewuDataset.open(path: url.path) }.value
        let view = document.view()
        let model = InspectorModel()
        for scale in [AxisScale.linear, .log10, .linear] {
            await model.load(
                dataset: document, view: view, column: 0,
                expanded: InspectorSection.distribution.mask,
                bins: 2, scale: scale)
            guard let histogram = model.distribution else {
                Issue.record("Missing distribution: \(model.errors)")
                return
            }
            #expect(histogram.scale == scale)
            #expect(histogram.nonPositiveCount == (scale == .log10 ? 2 : 0))
            #expect(histogram.counts.reduce(0, +) == (scale == .log10 ? 3 : 5))
        }
        #expect(view.rowCount() == 5)
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
