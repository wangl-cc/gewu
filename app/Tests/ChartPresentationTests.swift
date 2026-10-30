import Foundation
import Testing

/// Presentation preserves exact source text separately; compact axes must not
/// round large integers through Double or drop the shared temporal context.
@Suite
struct ChartPresentationTests {
    @Test
    static func labelsPreservePrecisionAndContext() throws {
        #expect(ChartLabels.number("9007199254740993") == "9.007…e15")
        #expect(ChartLabels.number("-0.0000000000012345") == "-1.234…e-12")
        #expect(ChartLabels.number("1234567890123456789012345678901234567890") == "1.234…e39")
        #expect(ChartLabels.number("1.234567890123456e100") == "1.234…e100")
        #expect(ChartLabels.number("not a numeric value") == "not a numeric value")
        let a = "2024-01-01T12:03:00.123456789Z"
        let b = "2024-01-01T12:03:00.123456790Z"
        let endpoints = ChartLabels.endpoints(a, b)
        #expect(endpoints.0 == "12:03:00.123456789" && endpoints.1 == "12:03:00.123456790")
        #expect(ChartLabels.context(a, b) == "2024-01-01 · UTC")
        let spanning = ChartLabels.endpoints("2024-01-01", "2024-12-31")
        #expect(spanning.0 == "01-01" && spanning.1 == "12-31")
        #expect(ChartLabels.context("2024-01-01", "2024-12-31") == "2024")
        let repeatedHour = ChartLabels.endpoints("2024-10-27T01:30:00+01:00", "2024-10-27T01:30:00+00:00")
        #expect(repeatedHour.0 == "01:30:00 +01:00" && repeatedHour.1 == "01:30:00 +00:00")
        #expect(ChartLabels.context("2024-01-01T10:00:00+08:00", "2024-01-01T11:00:00+08:00") == "2024-01-01 · +08:00")
        let datePoints = [
            ScatterPoint(sourceRow: 0, x: 0, y: 0, xLabel: "2024-01-01", yLabel: "2024-01-01T10:00:00.123456789+08:00"),
            ScatterPoint(sourceRow: 1, x: 1, y: 1, xLabel: "2024-01-02", yLabel: "2024-01-01T03:00:00.123456789"),
        ]
        let days = try #require(ScatterDates.make(points: datePoints, x: true, kind: .date))
        let timestamps = try #require(ScatterDates.make(points: datePoints, x: false, kind: .timestamp))
        #expect(days[1].timeIntervalSince(days[0]) == 86400)
        #expect(ScatterDates.context(days) == "2024 · UTC")
        #expect(abs(timestamps[1].timeIntervalSince(timestamps[0]) - 3600) < 0.001)
        #expect(ScatterDates.make(points: datePoints, x: true, kind: .integer) == nil)
    }
}
