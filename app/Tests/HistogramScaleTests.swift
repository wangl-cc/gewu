import Foundation
import Testing

struct HistogramScaleTests {
    @Test
    @MainActor
    static func changingBinningRefreshesTheInspectorWithoutFiltering() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).csv")
        try "value\n-1\n0\n1\n10\n100\n".write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        let document = try await Task.detached { try GewuDocument.open(path: url.path) }.value
        let model = InspectorModel()
        for scale in [AxisScale.linear, .log10, .linear] {
            await model.load(
                document: document, version: 0, column: 0, expanded: AnalysisSection.distribution.mask,
                bins: 2, scale: scale)
            guard case .distribution(let histogram) = model.results[.distribution] else {
                Issue.record("Missing distribution: \(model.errors)")
                return
            }
            #expect(histogram.scale == scale)
            #expect(histogram.nonPositiveCount == (scale == .log10 ? 2 : 0))
            #expect(histogram.counts.reduce(0, +) == (scale == .log10 ? 3 : 5))
        }
        #expect(document.currentView().rowCount == 5 && document.currentView().version == 0)
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
