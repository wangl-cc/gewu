import Foundation
import GewuKit
import Testing

@Suite struct RowSetTests {
    @Test func analysesDescribeTheFilteredRows() async throws {
        let file = try Fixture("city,score\nbj,10\nbj,20\nsh,30\nsh,\n")
        let session = try await file.open()
        let city = session.source.id("city")
        let score = session.source.id("score")
        let rows = try await session.apply(
            TableQuery(filters: [ColumnFilter(column: city, filter: exact("sh"))]), after: session.initialRows
        ).rowSet

        #expect(rows.count == 2)
        #expect(try await rows.nullCount(score) == 1)
        #expect(try await rows.statistics(score).mean == 30)
        let histogram = try await rows.distribution(score, bins: 4, scale: .linear)
        #expect(histogram.counts.reduce(0, +) == 1)
        #expect(try await rows.values(city).values.map(\.label) == ["sh"])
        #expect(try await rows.lengths(city, bins: 2).max == 2)
    }

    @Test func filterOptionsIgnoreTheEditedColumnsOwnFilter() async throws {
        let file = try Fixture("label,id\ncat,1\ndog,2\n")
        let session = try await file.open()
        let label = session.source.id("label")
        let id = session.source.id("id")
        let rows = try await session.apply(
            TableQuery(filters: [ColumnFilter(column: label, filter: exact("dog"))]), after: session.initialRows
        ).rowSet

        #expect(try await rows.filterOptions(for: label).values?.map(\.value) == ["cat", "dog"])
        #expect(try await rows.filterOptions(for: id).values?.map(\.value) == ["2"])
        let range = try #require(try await rows.filterDistribution(id, bins: 4, scale: .linear))
        #expect(range.ranges.count == range.histogram.counts.count)
    }

    @Test func scatterReportsTemporalCoordinatesInSeconds() async throws {
        let file = try Fixture("day,n\n2024-01-02,1\n2024-01-03,2\n")
        let session = try await file.open()
        let sample = try await session.initialRows.rowSet.scatter(
            x: session.source.id("day"), y: session.source.id("n"), limit: 16, xScale: .linear, yScale: .linear)
        #expect(sample.xAxis == .date && sample.yAxis == .number)
        let calendar = Calendar(identifier: .gregorian)
        let day = try #require(
            calendar.date(from: DateComponents(timeZone: .gmt, year: 2024, month: 1, day: 2)))
        #expect(sample.points.map(\.xDate) == [day, day.addingTimeInterval(86_400)])
        #expect(!sample.isSampled)
    }

    @Test func unsupportedAnalysesAreReported() async throws {
        let file = try Fixture("label\ncat\n")
        let session = try await file.open()
        await #expect(throws: GewuKitError.self) {
            try await session.initialRows.rowSet.statistics(session.source.id("label"))
        }
    }

    @Test func concurrentRequestsShareOneResult() async throws {
        let file = try Fixture("n\n" + (0..<10_000).map(String.init).joined(separator: "\n") + "\n")
        let session = try await file.open()
        let rows = session.initialRows.rowSet
        let column = session.source.id("n")
        async let first = rows.values(column)
        async let second = rows.values(column)
        #expect(try await first == second)
    }

    @Test func oneWaiterCancellingDoesNotCancelAnother() async throws {
        let file = try Fixture("n\n" + (0..<50_000).map(String.init).joined(separator: "\n") + "\n")
        let session = try await file.open()
        let rows = session.initialRows.rowSet
        let column = session.source.id("n")
        let cancelled = Task { try await rows.statistics(column) }
        let kept = Task { try await rows.statistics(column) }
        cancelled.cancel()
        await #expect(throws: CancellationError.self) { try await cancelled.value }
        #expect(try await kept.value.mean == 24_999.5)
    }
}
