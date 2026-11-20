import Foundation
import GewuKit
import Testing

/// Presentation preserves exact source text separately; compact axes must not
/// round large integers through Double or drop the shared temporal context.
@Suite
struct ChartPresentationTests {
    @Test
    static func logTicksStaySparseAndInOriginalUnits() {
        #expect(ScatterLogTicks.values([1e-4, 1e10], maximumCount: 3) == [1e-4, 1e3, 1e10])
        #expect(ScatterLogTicks.label(1e10) == "1e10")
        #expect(ScatterLogTicks.label(1e-4) == "1e-4")
        for values in [
            [2.0, 3.0], [7.0, 7.0],
            [Double.leastNonzeroMagnitude, Double.greatestFiniteMagnitude],
            [0, -1, .nan, .infinity],
        ] {
            let ticks = ScatterLogTicks.values(values, maximumCount: 4)
            #expect(ticks.count <= 4 && ticks.allSatisfy { $0.isFinite && $0 > 0 })
            #expect(zip(ticks, ticks.dropFirst()).allSatisfy { $0 < $1 })
            if let lower = values.filter({ $0.isFinite && $0 > 0 }).min(),
                let upper = values.filter({ $0.isFinite }).max()
            {
                #expect(!ticks.isEmpty && ticks.allSatisfy { $0 >= lower && $0 <= upper })
            } else {
                #expect(ticks.isEmpty)
            }
        }
    }

    @Test
    @MainActor
    static func scatterLogOptionsReachTheSampler() async throws {
        let session = try await TestTables.session("x,y\n-1,10\n1,0\n10,100\n100,1000\n")
        let rows = session.initialRows.rowSet
        let x = session.source.id("x")
        let y = session.source.id("y")
        let model = ScatterModel()
        func request(_ xScale: AxisScale, _ yScale: AxisScale, base: Int = 0) -> ScatterModel.Request {
            .init(rows: rows, x: x, y: y, limit: 2048, xScale: xScale, yScale: yScale, rowNumberBase: base, attempt: 0)
        }
        await model.load(request(.log10, .log10))
        let result = try #require(model.result)
        #expect(model.xScale == .log10 && model.yScale == .log10)
        #expect(result.nonPositiveRows == 2 && result.points.map(\.sourceRow) == [2, 3])
        #expect(result.points[0].x == 10 && result.points[0].xLabel == "10")
        await model.load(request(.linear, .linear))
        #expect(model.xScale == .linear && model.yScale == .linear && model.result?.eligibleRows == 4)
    }

    @Test
    @MainActor
    static func sourceRowAxisFollowsTheRowNumberBase() async throws {
        let session = try await TestTables.session("value\n5\n6\n")
        let model = ScatterModel()
        for base in [0, 1] {
            await model.load(
                .init(
                    rows: session.initialRows.rowSet, x: nil, y: session.source.id("value"), limit: 16,
                    xScale: .linear, yScale: .linear, rowNumberBase: base, attempt: 0))
            let point = try #require(model.result?.points.first)
            #expect(point.sourceRow == 0, "Presentation must not change source row identity")
            #expect(point.xLabel == String(base) && point.x == Double(base))
        }
    }

    @Test
    @MainActor
    static func cancelledScatterLeavesNoStaleState() async throws {
        let session = try await TestTables.session("value\n1\n2\n")
        let request = ScatterModel.Request(
            rows: session.initialRows.rowSet, x: nil, y: session.source.id("value"), limit: 16, xScale: .linear,
            yScale: .linear, rowNumberBase: 0, attempt: 0)
        let model = ScatterModel()
        let obsolete = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            await model.load(request)
        }
        await obsolete.value
        #expect(model.result == nil && !model.loading && model.error == nil)
        await model.load(request)
        #expect(model.result?.points.count == 2 && model.totalRows == 2 && !model.loading)
    }

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
        // Temporal coordinates arrive in seconds since 1970.
        let day = 1_704_067_200.0  // 2024-01-01T00:00:00Z
        let datePoints = [
            ScatterPoint(
                sourceRow: 0, x: day, y: day + 7200, xLabel: "2024-01-01", yLabel: "2024-01-01T10:00:00+08:00"),
            ScatterPoint(
                sourceRow: 1, x: day + 86400, y: day + 10800, xLabel: "2024-01-02", yLabel: "2024-01-01T03:00:00"),
        ]
        let days = try #require(ScatterDates.make(points: datePoints, x: true, kind: .date))
        let timestamps = try #require(ScatterDates.make(points: datePoints, x: false, kind: .dateTime))
        #expect(days[1].timeIntervalSince(days[0]) == 86400)
        #expect(ScatterDates.context(days) == "2024 · UTC")
        #expect(timestamps[1].timeIntervalSince(timestamps[0]) == 3600)
        #expect(ScatterDates.make(points: datePoints, x: true, kind: .number) == nil)
    }
}
