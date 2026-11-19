import Foundation
import Observation
import Testing
import os

struct InspectorModelTests {
    private static func document(_ csv: String) throws -> GewuDataset {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).csv")
        try csv.write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        return try GewuDataset.open(path: url.path)
    }

    @MainActor
    private static func viewer(
        _ dataset: GewuDataset, _ filtered: GewuFilteredView, analysisWorker: OperationQueue? = nil
    ) -> Viewer {
        Viewer(dataset: dataset, filtered: filtered, ordered: filtered.unsorted(), analysisWorker: analysisWorker)
    }

    @Test
    @MainActor
    static func typedCachesInvalidateOnlyForRelevantInputs() async throws {
        let doc = try document("number,text\n1,a\n2,bb\n3,ccc\n4,dddd\n")
        let columns = doc.info().columns
        let filtered = doc.unfiltered()
        let model = InspectorModel()
        let all =
            InspectorSection.overview.mask | InspectorSection.statistics.mask
            | InspectorSection.distribution.mask | InspectorSection.values.mask | InspectorSection.lengths.mask
        await model.load(
            column: columns[0], dataset: doc, viewer: viewer(doc, filtered), expanded: all, bins: 2)
        #expect(model.nullCount == 0 && model.rowCount == 4)
        #expect(model.statistics?.mean == 2.5)
        #expect(model.values?.distinctCount == 4)
        #expect(model.distribution?.counts.count == 2)

        await model.load(
            column: columns[0], dataset: doc, viewer: viewer(doc, filtered), expanded: 0, bins: 3)
        #expect(model.distribution == nil)
        #expect(model.nullCount != nil && model.statistics != nil && model.values != nil)
        await model.load(
            column: columns[0], dataset: doc, viewer: viewer(doc, filtered), expanded: all, bins: 3)
        #expect(model.distribution?.counts.count == 3)
        await model.load(
            column: columns[0], dataset: doc, viewer: viewer(doc, filtered), expanded: 0, bins: 3,
            scale: .log10)
        #expect(model.distribution == nil)
        #expect(model.nullCount != nil && model.statistics != nil && model.values != nil)

        await model.load(
            column: columns[1], dataset: doc, viewer: viewer(doc, filtered), expanded: all, bins: 2)
        #expect(model.statistics == nil && model.distribution == nil)
        #expect(model.lengths?.histogram.counts.count == 2)
        await model.load(
            column: columns[1], dataset: doc, viewer: viewer(doc, filtered), expanded: 0, bins: 2,
            scale: .log10)
        #expect(model.lengths != nil, "Numeric scale does not invalidate length statistics")
        await model.load(
            column: columns[1], dataset: doc, viewer: viewer(doc, filtered), expanded: 0, bins: 3,
            scale: .log10)
        #expect(model.lengths == nil)
        #expect(model.nullCount != nil && model.values != nil)

        let replacement = try document("number\n100\n")
        await model.load(
            column: replacement.info().columns[0], dataset: replacement,
            viewer: viewer(replacement, replacement.unfiltered()),
            expanded: InspectorSection.statistics.mask)
        #expect(model.statistics?.mean == 100)
        #expect(model.nullCount == nil && model.values == nil && model.lengths == nil)
    }

    @Test
    @MainActor
    static func cancelledTaskDoesNotPublishAndAllowsReplacement() async throws {
        let doc = try document("first,second\n1,100\n2,200\n")
        let columns = doc.info().columns
        let filtered = doc.unfiltered()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let model = InspectorModel()
        let v = viewer(doc, filtered, analysisWorker: worker)
        let task = Task {
            await model.load(
                column: columns[0], dataset: doc, viewer: v,
                expanded: InspectorSection.statistics.mask)
        }
        while model.loading != .statistics || worker.operations.filter({ !$0.isFinished }).isEmpty {
            await Task.yield()
        }
        task.cancel()
        worker.isSuspended = false
        await task.value
        #expect(model.statistics == nil && model.loading == nil && model.errors.isEmpty)
        await model.load(
            column: columns[1], dataset: doc, viewer: v,
            expanded: InspectorSection.statistics.mask)
        #expect(model.statistics?.mean == 150)
        #expect(model.matches(filtered: filtered, column: columns[1].id))
    }

    @Test
    @MainActor
    static func replacingViewClearsCachesAndRetainsItsOwnRowCount() async throws {
        let doc = try document("number\n1\n2\n")
        let col0 = doc.info().columns[0]
        let unfiltered = doc.unfiltered()
        let model = InspectorModel()
        await model.load(
            column: col0, dataset: doc, viewer: viewer(doc, unfiltered), expanded: InspectorSection.statistics.mask)
        let filtered = try doc.filter(filters: [
            ColumnFilter(
                column: col0.id,
                filter: Filter(condition: .range(min: "2", max: nil), includeNulls: false))
        ])
        await model.load(
            column: col0, dataset: doc, viewer: viewer(doc, filtered), expanded: InspectorSection.overview.mask)
        #expect(model.statistics == nil && model.nullCount == 0 && model.rowCount == 1)
        await model.load(
            column: col0, dataset: doc, viewer: viewer(doc, unfiltered), expanded: InspectorSection.statistics.mask)
        #expect(model.statistics?.mean == 1.5 && model.nullCount == nil && model.rowCount == 2)
        #expect(model.loading == nil && model.errors.isEmpty && model.error == nil)
    }

    @Test
    @MainActor
    static func matchesTracksContextChangesThroughObservation() async throws {
        let doc = try document("first,second\n1,10\n2,20\n")
        let columns = doc.info().columns
        let filtered = doc.unfiltered()
        let v = viewer(doc, filtered)
        let model = InspectorModel()
        await model.load(column: columns[0], dataset: doc, viewer: v, expanded: 0)

        let changes = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            let ready = model.matches(filtered: filtered, column: columns[1].id)
            #expect(!ready)
        } onChange: {
            changes.withLock { $0 += 1 }
        }

        await model.load(column: columns[1], dataset: doc, viewer: v, expanded: 0)
        #expect(changes.withLock { $0 } == 1)
        #expect(model.matches(filtered: filtered, column: columns[1].id))
    }
}
