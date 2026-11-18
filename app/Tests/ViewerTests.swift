import Foundation
import Testing

@Suite @MainActor
struct ViewerTests {
    private static func dataset() throws -> GewuDataset {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("viewer-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try "city,district\nbj,haidian\nbj,chaoyang\nsh,pudong\nsh,xuhui\n"
            .write(to: url, atomically: true, encoding: .utf8)
        return try GewuDataset.open(path: url.path)
    }

    private static func exact(_ value: String) -> Filter {
        Filter(condition: .exact(values: [value], exclude: false), includeNulls: false)
    }

    private static func spec(city: String?, district: String?) -> QuerySpec {
        QuerySpec(columns: [
            ColSpec(column: 0, sort: nil, filter: city.map(exact)),
            ColSpec(column: 1, sort: nil, filter: district.map(exact)),
        ])
    }

    @Test func queryDoesNotPublishBeforeTheWindowCommits() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        let spec = Self.spec(city: "sh", district: nil)
        let result = try await viewer.query(spec)
        #expect(viewer.applied.view === view)
        #expect(viewer.applied.spec == Self.spec(city: nil, district: nil))
        viewer.apply(result, spec: spec)
        #expect(viewer.applied.spec == Self.spec(city: "sh", district: nil))
        #expect(viewer.applied.view.rowCount() == 2)
    }

    @Test func failedApplyKeepsTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let initial = dataset.view()
        let viewer = Viewer(dataset: dataset, view: initial, spec: Self.spec(city: nil, district: nil))
        var invalid = Self.spec(city: nil, district: nil)
        invalid.columns[0].filter = Filter(condition: .range(min: "1", max: nil), includeNulls: false)
        do {
            _ = try await viewer.query(invalid)
            Issue.record("A text range must fail")
        } catch {}
        #expect(viewer.applied.view === initial)
    }

    @Test func facetsAreTemporaryAndChoicesFollowTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        try await Self.apply(Self.spec(city: "bj", district: "haidian"), to: viewer)
        let facet = try await viewer.facet(1)
        #expect(try await viewer.facet(1) !== facet, "Completed facets must not retain their row arrays in the cache")
        #expect(facet.rowCount() == 2, "The district facet keeps city = bj only")
        weak var released: GewuView?
        released = try await viewer.facet(1)
        #expect(released == nil, "The Viewer must release a completed facet after its last consumer releases it")
        let choices = try await viewer.filterChoices(1)
        #expect(choices.values?.map(\.label).sorted() == ["chaoyang", "haidian"])

        // Column order does not affect any cached result.
        var reordered = viewer.applied.spec
        reordered.columns.reverse()
        let rowView = viewer.applied.rowView
        try viewer.reorder([1, 0], spec: reordered)
        #expect(viewer.applied.rowView === rowView)
        #expect(viewer.applied.view !== rowView)
        #expect(try await viewer.filterChoices(1) == choices)

        try await Self.apply(Self.spec(city: "sh", district: "pudong"), to: viewer)
        #expect(viewer.applied.rowView !== rowView)
        #expect(viewer.applied.rowView === viewer.applied.view)
        let replaced = try await viewer.facet(1)
        #expect(replaced !== facet)
        let labels = try await viewer.filterChoices(1).values?.map(\.label)
        #expect(labels == ["pudong", "xuhui"])
    }

    /// Hold scans before they enter Core, so cancellation and replacement do not depend on scan speed.
    private static func queued(on worker: OperationQueue, count: Int = 1) async throws {
        while worker.operations.filter({ !$0.isFinished }).count < count {
            try Task.checkCancellation()
            await Task.yield()
        }
    }

    private static func expectCancelled<T>(_ task: Task<T, Error>) async {
        do {
            _ = try await task.value
            Issue.record("Superseded work must not return a result")
        } catch {
            #expect(error.isCancellation)
        }
    }

    private static func apply(_ spec: QuerySpec, to viewer: Viewer) async throws {
        let view = try await viewer.query(spec)
        viewer.apply(view, spec: spec)
    }

    @Test func consumerCancellationStopsQueuedWorkAndCompletedCacheKeepsLatestParameters() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: Self.spec(city: nil, district: nil), analysisWorker: worker)
        #expect(worker.maxConcurrentOperationCount == 1)
        let older = Task { try await viewer.lengths(1, bins: 2) }
        try await Self.queued(on: worker)
        let operation = try #require(worker.operations.first)
        let newer = Task { try await viewer.lengths(1, bins: 3) }
        try await Self.queued(on: worker, count: 2)
        #expect(!operation.isCancelled, "Another consumer's parameters must not cancel this consumer")
        older.cancel()
        while !operation.isCancelled { await Task.yield() }
        worker.isSuspended = false
        await Self.expectCancelled(older)
        let result = try await newer.value
        worker.isSuspended = true
        #expect(result == (try await viewer.lengths(1, bins: 3)))

        // A previous parameter set must now require new work, rather than hitting history.
        let previous = Task { try await viewer.lengths(1, bins: 2) }
        try await Self.queued(on: worker)
        viewer.close()
        worker.isSuspended = false
        await Self.expectCancelled(previous)
    }

    @Test func completedResultBudgetEvictsOldColumns() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("cache-budget-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        let header = (0..<33).map { "c\($0)" }.joined(separator: ",")
        let row = Array(repeating: "1", count: 33).joined(separator: ",")
        try "\(header)\n\(row)\n".write(to: url, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: url.path)
        let worker = OperationQueue()
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: QuerySpec(columns: []), analysisWorker: worker)
        for column in 0..<33 {
            #expect(try await viewer.countNulls(UInt32(column)) == 0)
        }
        worker.isSuspended = true
        #expect(try await viewer.countNulls(32) == 0, "A completed cached result needs no worker")
        let evicted = Task { try await viewer.countNulls(0) }
        try await Self.queued(on: worker)
        evicted.cancel()
        worker.isSuspended = false
        await Self.expectCancelled(evicted)
    }

    @Test func olderParametersFinishingLastDoNotReplaceTheLatestCachedResult() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: Self.spec(city: nil, district: nil), analysisWorker: worker)
        let older = Task { try await viewer.lengths(1, bins: 2) }
        try await Self.queued(on: worker)
        let olderOperation = try #require(worker.operations.first)
        let newer = Task { try await viewer.lengths(1, bins: 3) }
        try await Self.queued(on: worker, count: 2)
        let newerOperation = try #require(worker.operations.first { $0 !== olderOperation })
        olderOperation.addDependency(newerOperation)
        worker.isSuspended = false
        let latest = try await newer.value
        _ = try await older.value
        worker.isSuspended = true
        #expect(try await viewer.lengths(1, bins: 3) == latest)
        #expect(worker.operations.allSatisfy { $0.isFinished }, "Cache hits must not queue new scans")
    }

    @Test func replacementCancelsCachedWorkAndOneConsumerCannotCancelAnother() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: Self.spec(city: nil, district: nil), analysisWorker: worker)
        let first = Task { try await viewer.countNulls(0) }
        try await Self.queued(on: worker)
        let firstOperation = try #require(worker.operations.first)
        let other = Task { try await viewer.countNulls(0) }
        try await Self.queued(on: worker, count: 2)
        #expect(worker.operations.filter { !$0.isFinished }.count == 2, "Pending computations belong to their callers")
        first.cancel()
        #expect(firstOperation.isCancelled)
        worker.isSuspended = false
        await Self.expectCancelled(first)
        #expect(try await other.value == 0)

        worker.isSuspended = true
        let old = Task { try await viewer.filterChoices(1) }
        try await Self.queued(on: worker)
        try await Self.apply(Self.spec(city: "sh", district: nil), to: viewer)
        worker.isSuspended = false
        await Self.expectCancelled(old)
        #expect(try await viewer.filterChoices(1).values?.map(\.label) == ["pudong", "xuhui"])
    }

    @Test func closingInspectorCancelsItsQueuedScan() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: Self.spec(city: nil, district: nil), analysisWorker: worker)
        let inspector = InspectorModel()
        let request = Task {
            await inspector.load(
                dataset: dataset, viewer: viewer, column: 0, expanded: InspectorSection.overview.mask)
        }
        try await Self.queued(on: worker)
        let operation = try #require(worker.operations.first)
        inspector.cancel()
        #expect(operation.isCancelled)
        worker.isSuspended = false
        await request.value
        #expect(inspector.nullCount == nil && inspector.loading == nil)
    }

    @Test func queryCancellationBelongsToItsCallingTask() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        let pending = Task { try await viewer.query(Self.spec(city: "bj", district: nil), debounce: true) }
        await Task.yield()
        pending.cancel()
        await Self.expectCancelled(pending)
        #expect(viewer.applied.spec == Self.spec(city: nil, district: nil))
    }
}
