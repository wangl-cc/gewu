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

    private static func numericDataset() throws -> GewuDataset {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("viewer-num-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        try "city,score\nbj,10\nbj,20\nsh,30\nsh,40\n"
            .write(to: url, atomically: true, encoding: .utf8)
        return try GewuDataset.open(path: url.path)
    }

    private static func exact(_ value: String) -> Filter {
        Filter(condition: .exact(values: [value], exclude: false), includeNulls: false)
    }

    private static func makeViewer(
        _ dataset: GewuDataset, analysisWorker: OperationQueue? = nil
    ) -> Viewer {
        let filtered = dataset.unfiltered()
        return Viewer(
            dataset: dataset, filtered: filtered, ordered: filtered.unsorted(), analysisWorker: analysisWorker)
    }

    private static func filters(city: String?, district: String?) -> [ColumnFilter] {
        var result: [ColumnFilter] = []
        if let city { result.append(ColumnFilter(column: ColumnId(raw: 0), filter: exact(city))) }
        if let district { result.append(ColumnFilter(column: ColumnId(raw: 1), filter: exact(district))) }
        return result
    }

    @Test func queryDoesNotPublishBeforeTheWindowCommits() async throws {
        let dataset = try Self.dataset()
        let filtered = dataset.unfiltered()
        let ordered = filtered.unsorted()
        let viewer = Viewer(dataset: dataset, filtered: filtered, ordered: ordered)
        let targetFilters = Self.filters(city: "sh", district: nil)
        let nextFiltered = try await viewer.filter(targetFilters)
        let nextOrdered = try await viewer.sort([], on: nextFiltered)
        #expect(viewer.filtered === filtered)
        #expect(viewer.ordered === ordered)
        #expect(viewer.appliedFilters.isEmpty)
        viewer.apply(filtered: nextFiltered, ordered: nextOrdered, filters: targetFilters, sorts: [])
        #expect(viewer.appliedFilters == targetFilters)
        #expect(viewer.filtered.rowCount() == 2)
        #expect(viewer.ordered.rowCount() == 2)
    }

    @Test func failedApplyKeepsTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let initial = dataset.unfiltered()
        let viewer = Viewer(dataset: dataset, filtered: initial, ordered: initial.unsorted())
        let invalid = [
            ColumnFilter(
                column: ColumnId(raw: 0),
                filter: Filter(condition: .range(min: "1", max: nil), includeNulls: false))
        ]
        do {
            _ = try await viewer.filter(invalid)
            Issue.record("A text range must fail")
        } catch {}
        #expect(viewer.filtered === initial)
    }

    @Test func facetsAreTemporaryAndChoicesFollowTheAppliedView() async throws {
        let dataset = try Self.dataset()
        let viewer = Self.makeViewer(dataset)
        let col1 = ColumnId(raw: 1)
        try await Self.apply(Self.filters(city: "bj", district: "haidian"), sorts: [], to: viewer)
        let facet = try await viewer.facet(col1)
        #expect(
            try await viewer.facet(col1) !== facet, "Completed facets must not retain their row arrays in the cache")
        #expect(facet.rowCount() == 2, "The district facet keeps city = bj only")
        weak var released: GewuFilteredView?
        released = try await viewer.facet(col1)
        #expect(released == nil, "The Viewer must release a completed facet after its last consumer releases it")
        let choices = try await viewer.filterChoices(col1)
        #expect(choices.values?.map(\.label).sorted() == ["chaoyang", "haidian"])

        // Pure sorting keeps `filtered` and its cached analysis results.
        let beforeFiltered = viewer.filtered
        let sorts = [ColumnSort(column: col1, descending: true)]
        let sorted = try await viewer.sort(sorts, on: beforeFiltered)
        viewer.apply(ordered: sorted, sorts: sorts)
        #expect(viewer.filtered === beforeFiltered)
        #expect(viewer.ordered === sorted)
        #expect(try await viewer.filterChoices(col1) == choices)

        try await Self.apply(Self.filters(city: "sh", district: "pudong"), sorts: [], to: viewer)
        #expect(viewer.filtered !== beforeFiltered)
        let replaced = try await viewer.facet(col1)
        #expect(replaced !== facet)
        let labels = try await viewer.filterChoices(col1).values?.map(\.label)
        #expect(labels == ["pudong", "xuhui"])
    }

    @Test func pureSortRetainsFilteredViewAndEpochCachesWhileFilterReplacesBoth() async throws {
        let dataset = try Self.numericDataset()
        let worker = OperationQueue()
        defer { worker.isSuspended = false }
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        let cityCol = ColumnId(raw: 0)
        let scoreCol = ColumnId(raw: 1)

        let initialFiltered = viewer.filtered
        let stats = try await viewer.statistics(scoreCol)
        let choices = try await viewer.filterChoices(cityCol)
        #expect(stats.mean == 25)
        #expect(choices.values?.map(\.label).sorted() == ["bj", "sh"])

        // Pure sort keeps `viewer.filtered ===` and hits the Epoch cache with worker suspended.
        let sorts = [ColumnSort(column: scoreCol, descending: true)]
        let sortedOrdered = try await viewer.sort(sorts, on: initialFiltered)
        viewer.apply(ordered: sortedOrdered, sorts: sorts)
        #expect(viewer.filtered === initialFiltered)
        #expect(viewer.ordered === sortedOrdered)
        #expect(viewer.appliedSorts == sorts)

        worker.isSuspended = true
        #expect(try await viewer.statistics(scoreCol) == stats)
        #expect(try await viewer.filterChoices(cityCol) == choices)
        worker.isSuspended = false

        // Changing filters replaces `viewer.filtered` and invalidates Epoch caches.
        let nextFilters = [ColumnFilter(column: cityCol, filter: Self.exact("sh"))]
        let nextFiltered = try await viewer.filter(nextFilters)
        let nextOrdered = try await viewer.sort(sorts, on: nextFiltered)
        viewer.apply(filtered: nextFiltered, ordered: nextOrdered, filters: nextFilters, sorts: sorts)
        #expect(viewer.filtered === nextFiltered)
        #expect(viewer.filtered !== initialFiltered)

        worker.isSuspended = true
        let recomputedStats = Task { try await viewer.statistics(scoreCol) }
        try await Self.queued(on: worker)
        worker.isSuspended = false
        #expect(try await recomputedStats.value.mean == 35)
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

    private static func apply(
        _ filters: [ColumnFilter], sorts: [ColumnSort], to viewer: Viewer
    ) async throws {
        let filtered = try await viewer.filter(filters)
        let ordered = try await viewer.sort(sorts, on: filtered)
        viewer.apply(filtered: filtered, ordered: ordered, filters: filters, sorts: sorts)
    }

    @Test func consumerCancellationStopsQueuedWorkAndCompletedCacheKeepsLatestParameters() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        let col1 = ColumnId(raw: 1)
        #expect(worker.maxConcurrentOperationCount == 1)
        let older = Task { try await viewer.lengths(col1, bins: 2) }
        try await Self.queued(on: worker)
        let operation = try #require(worker.operations.first)
        let newer = Task { try await viewer.lengths(col1, bins: 3) }
        try await Self.queued(on: worker, count: 2)
        #expect(!operation.isCancelled, "Another consumer's parameters must not cancel this consumer")
        older.cancel()
        while !operation.isCancelled { await Task.yield() }
        worker.isSuspended = false
        await Self.expectCancelled(older)
        let result = try await newer.value
        worker.isSuspended = true
        #expect(result == (try await viewer.lengths(col1, bins: 3)))

        // A previous parameter set must now require new work, rather than hitting history.
        let previous = Task { try await viewer.lengths(col1, bins: 2) }
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
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        for column in 0..<33 {
            #expect(try await viewer.countNulls(ColumnId(raw: UInt32(column))) == 0)
        }
        worker.isSuspended = true
        #expect(try await viewer.countNulls(ColumnId(raw: 32)) == 0, "A completed cached result needs no worker")
        let evicted = Task { try await viewer.countNulls(ColumnId(raw: 0)) }
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
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        let col1 = ColumnId(raw: 1)
        let older = Task { try await viewer.lengths(col1, bins: 2) }
        try await Self.queued(on: worker)
        let olderOperation = try #require(worker.operations.first)
        let newer = Task { try await viewer.lengths(col1, bins: 3) }
        try await Self.queued(on: worker, count: 2)
        let newerOperation = try #require(worker.operations.first { $0 !== olderOperation })
        olderOperation.addDependency(newerOperation)
        worker.isSuspended = false
        let latest = try await newer.value
        _ = try await older.value
        worker.isSuspended = true
        #expect(try await viewer.lengths(col1, bins: 3) == latest)
        #expect(worker.operations.allSatisfy { $0.isFinished }, "Cache hits must not queue new scans")
    }

    @Test func replacementCancelsCachedWorkAndOneConsumerCannotCancelAnother() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        let col0 = ColumnId(raw: 0)
        let col1 = ColumnId(raw: 1)
        let first = Task { try await viewer.countNulls(col0) }
        try await Self.queued(on: worker)
        let firstOperation = try #require(worker.operations.first)
        let other = Task { try await viewer.countNulls(col0) }
        try await Self.queued(on: worker, count: 2)
        #expect(worker.operations.filter { !$0.isFinished }.count == 2, "Pending computations belong to their callers")
        first.cancel()
        #expect(firstOperation.isCancelled)
        worker.isSuspended = false
        await Self.expectCancelled(first)
        #expect(try await other.value == 0)

        worker.isSuspended = true
        let old = Task { try await viewer.filterChoices(col1) }
        try await Self.queued(on: worker)
        try await Self.apply(Self.filters(city: "sh", district: nil), sorts: [], to: viewer)
        worker.isSuspended = false
        await Self.expectCancelled(old)
        #expect(try await viewer.filterChoices(col1).values?.map(\.label) == ["pudong", "xuhui"])
    }

    @Test func closingInspectorCancelsItsQueuedScan() async throws {
        let dataset = try Self.dataset()
        let worker = OperationQueue()
        worker.isSuspended = true
        defer { worker.isSuspended = false }
        let viewer = Self.makeViewer(dataset, analysisWorker: worker)
        let inspector = InspectorModel()
        let col0 = dataset.info().columns[0]
        let request = Task {
            await inspector.load(
                column: col0, dataset: dataset, viewer: viewer, expanded: InspectorSection.overview.mask)
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
        let filtered = dataset.unfiltered()
        let viewer = Viewer(dataset: dataset, filtered: filtered, ordered: filtered.unsorted())
        let pending = Task { try await viewer.filter(Self.filters(city: "bj", district: nil), debounce: true) }
        await Task.yield()
        pending.cancel()
        await Self.expectCancelled(pending)
        #expect(viewer.appliedFilters.isEmpty)
        #expect(viewer.filtered === filtered)
    }
}
