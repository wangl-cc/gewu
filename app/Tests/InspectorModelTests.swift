import Foundation
import GewuKit
import Observation
import Testing
import os

@MainActor
struct InspectorModelTests {
    private static let all: Set<InspectorSection> = [.overview, .statistics, .distribution, .values, .lengths]

    private static func request(
        _ rows: RowSet, _ column: ColumnID, _ expanded: Set<InspectorSection>, bins: UInt32 = 40,
        scale: AxisScale = .linear
    ) -> InspectorModel.Request {
        .init(rows: rows, column: column, expanded: expanded, bins: bins, scale: scale, attempt: 0)
    }

    @Test
    static func resultsResetOnlyForTheInputsTheyDependOn() async throws {
        let session = try await TestTables.session("number,text\n1,a\n2,bb\n3,ccc\n4,dddd\n")
        let rows = session.initialRows.rowSet
        let number = session.source.id("number")
        let text = session.source.id("text")
        let model = InspectorModel()
        await model.load(request(rows, number, all, bins: 2))
        #expect(model.nullCount == 0 && model.rows?.count == 4)
        #expect(model.statistics?.mean == 2.5)
        #expect(model.values?.distinctCount == 4)
        #expect(model.distribution?.counts.count == 2)

        await model.load(request(rows, number, [], bins: 3))
        #expect(model.distribution == nil, "Bins replace the distribution")
        #expect(model.nullCount != nil && model.statistics != nil && model.values != nil)
        await model.load(request(rows, number, all, bins: 3))
        #expect(model.distribution?.counts.count == 3)
        await model.load(request(rows, number, [], bins: 3, scale: .log10))
        #expect(model.distribution == nil, "Scale replaces the distribution")
        #expect(model.nullCount != nil && model.statistics != nil && model.values != nil)

        await model.load(request(rows, text, all, bins: 2))
        #expect(model.statistics == nil && model.distribution == nil)
        #expect(model.lengths?.histogram.counts.count == 2)
        await model.load(request(rows, text, [], bins: 2, scale: .log10))
        #expect(model.lengths != nil, "Numeric scale does not invalidate length statistics")
        await model.load(request(rows, text, [], bins: 3, scale: .log10))
        #expect(model.lengths == nil)
        #expect(model.nullCount != nil && model.values != nil)
    }

    @Test
    static func newRowsReplaceResultsAndTheirRowCount() async throws {
        let session = try await TestTables.session("number\n1\n2\n")
        let number = session.source.id("number")
        let all = session.initialRows
        let filtered = try await session.apply(
            TableQuery(filters: [
                ColumnFilter(column: number, filter: .condition(.range(min: "2", max: nil), includeNulls: false))
            ]), after: all)
        let model = InspectorModel()
        await model.load(request(all.rowSet, number, [.statistics]))
        await model.load(request(filtered.rowSet, number, [.overview]))
        #expect(model.statistics == nil && model.nullCount == 0 && model.rows?.count == 1)
        await model.load(request(all.rowSet, number, [.statistics]))
        #expect(model.statistics?.mean == 1.5 && model.nullCount == nil && model.rows?.count == 2)
        #expect(model.loading == nil && model.errors.isEmpty)
    }

    @Test
    static func cancelledLoadsPublishNothing() async throws {
        let session = try await TestTables.session("first,second\n1,100\n2,200\n")
        let rows = session.initialRows.rowSet
        let model = InspectorModel()
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            await model.load(request(rows, session.source.id("first"), [.statistics]))
        }
        await cancelled.value
        #expect(model.statistics == nil && model.loading == nil && model.errors.isEmpty)
        await model.load(request(rows, session.source.id("second"), [.statistics]))
        #expect(model.statistics?.mean == 150)
        #expect(model.shows(rows, column: session.source.id("second")))
    }

    @Test
    static func onlySectionsTheColumnSupportsAreLoaded() async throws {
        let session = try await TestTables.session("text\na\n")
        let model = InspectorModel()
        await model.load(request(session.initialRows.rowSet, session.source.id("text"), [.overview, .statistics]))
        #expect(model.sections == [.overview, .values, .lengths])
        #expect(model.nullCount == 0 && model.statistics == nil && model.errors.isEmpty)
    }

    @Test
    static func showsTracksContextChangesThroughObservation() async throws {
        let session = try await TestTables.session("first,second\n1,10\n2,20\n")
        let rows = session.initialRows.rowSet
        let first = session.source.id("first")
        let second = session.source.id("second")
        let model = InspectorModel()
        await model.load(request(rows, first, []))

        let changes = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            #expect(!model.shows(rows, column: second))
        } onChange: {
            changes.withLock { $0 += 1 }
        }
        await model.load(request(rows, second, []))
        #expect(changes.withLock { $0 } >= 1)
        #expect(model.shows(rows, column: second))
    }

    @Test
    static func settingsKeepUserExpansionAndLinearBinningWhereLogIsUnsupported() {
        let settings = InspectorSettings()
        #expect(settings.expandedSections(defaultingToDistribution: true) == [.overview, .statistics, .distribution])
        #expect(settings.expandedSections(defaultingToDistribution: false) == [.overview, .values])
        settings.setExpanded(.values, true)
        settings.setExpanded(.statistics, false)
        #expect(settings.expandedSections(defaultingToDistribution: true) == [.overview, .distribution, .values])
        settings.histogramScale = .log10
        #expect(settings.binningScale(logarithmic: false) == .linear)
        #expect(settings.binningScale(logarithmic: true) == .log10)
    }
}
