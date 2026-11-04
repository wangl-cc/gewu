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

    @Test func newerApplySupersedesAnOlderOne() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        let older = Task { await viewer.apply(Self.spec(city: "bj", district: nil), debounce: true) }
        // Let the older request start first; the main actor runs tasks in order.
        await Task.yield()
        let newer = await viewer.apply(Self.spec(city: "sh", district: nil))
        guard case .applied = newer else {
            Issue.record("The newest request must apply")
            return
        }
        guard case .superseded = await older.value else {
            Issue.record("The older request must not apply")
            return
        }
        #expect(viewer.applied.spec == Self.spec(city: "sh", district: nil))
        #expect(viewer.applied.view.rowCount() == 2)
    }

    @Test func failedApplyKeepsTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let initial = dataset.view()
        let viewer = Viewer(dataset: dataset, view: initial, spec: Self.spec(city: nil, district: nil))
        var invalid = Self.spec(city: nil, district: nil)
        invalid.columns[0].filter = Filter(condition: .range(min: "1", max: nil), includeNulls: false)
        guard case .failed = await viewer.apply(invalid) else {
            Issue.record("A text range must fail")
            return
        }
        #expect(viewer.applied.view === initial)
    }

    @Test func facetsAreTemporaryAndChoicesFollowTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        _ = await viewer.apply(Self.spec(city: "bj", district: "haidian"))
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
        try viewer.reorder([1, 0], spec: reordered)
        #expect(try await viewer.filterChoices(1) == choices)

        _ = await viewer.apply(Self.spec(city: "sh", district: "pudong"))
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

    @Test func changedParametersCancelQueuedWorkAndKeepOnlyTheLatestResult() async throws {
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
        while !operation.isCancelled { await Task.yield() }
        worker.isSuspended = false
        await Self.expectCancelled(older)
        let result = try await newer.value
        #expect(result == (try await viewer.lengths(1, bins: 3)))

        // A previous parameter set must now require new work, rather than hitting history.
        worker.isSuspended = true
        let previous = Task { try await viewer.lengths(1, bins: 2) }
        try await Self.queued(on: worker)
        viewer.close()
        worker.isSuspended = false
        await Self.expectCancelled(previous)
    }

    @Test func resultBudgetEvictsOldColumnsIncludingQueuedWork() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("cache-budget-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        let header = (0..<33).map { "c\($0)" }.joined(separator: ",")
        let row = Array(repeating: "1", count: 33).joined(separator: ",")
        try "\(header)\n\(row)\n".write(to: url, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: url.path)
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Viewer(
            dataset: dataset, view: dataset.view(), spec: QuerySpec(columns: []), analysisWorker: worker)
        var tasks: [Task<UInt64, Error>] = []
        for column in 0..<32 {
            tasks.append(Task { try await viewer.countNulls(UInt32(column)) })
            try await Self.queued(on: worker, count: column + 1)
        }
        let operations = worker.operations
        tasks.append(Task { try await viewer.countNulls(32) })
        while !operations.contains(where: \.isCancelled) { await Task.yield() }
        worker.isSuspended = false
        await Self.expectCancelled(tasks[0])
        for task in tasks.dropFirst() { #expect(try await task.value == 0) }
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
        let other = Task { try await viewer.countNulls(0) }
        first.cancel()
        worker.isSuspended = false
        await Self.expectCancelled(first)
        #expect(try await other.value == 0)

        worker.isSuspended = true
        let old = Task { try await viewer.filterChoices(1) }
        try await Self.queued(on: worker)
        _ = await viewer.apply(Self.spec(city: "sh", district: nil))
        worker.isSuspended = false
        await Self.expectCancelled(old)
        #expect(try await viewer.filterChoices(1).values?.map(\.label) == ["pudong", "xuhui"])
    }

    @Test func closingSupersedesPendingQueries() async throws {
        let dataset = try Self.dataset()
        let view = dataset.view()
        let viewer = Viewer(dataset: dataset, view: view, spec: Self.spec(city: nil, district: nil))
        let pending = Task { await viewer.apply(Self.spec(city: "bj", district: nil), debounce: true) }
        await Task.yield()
        viewer.close()
        guard case .superseded = await pending.value else {
            Issue.record("Closing must stop queries")
            return
        }
        #expect(viewer.applied.spec == Self.spec(city: nil, district: nil))
    }
}
