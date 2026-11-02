import Combine
import Foundation
import Testing

struct InspectorModelTests {
    private static func document(_ csv: String) throws -> GewuSession {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).csv")
        try csv.write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        return try GewuSession.open(path: url.path)
    }

    @Test
    @MainActor
    static func typedCachesInvalidateOnlyForRelevantInputs() async throws {
        let doc = try document("number,text\n1,a\n2,bb\n3,ccc\n4,dddd\n")
        let model = InspectorModel()
        let all =
            AnalysisSection.overview.mask | AnalysisSection.statistics.mask
            | AnalysisSection.distribution.mask | AnalysisSection.values.mask | AnalysisSection.lengths.mask
        await model.load(document: doc, version: 0, column: 0, expanded: all, bins: 2)
        #expect(model.overview?.rows == 4)
        #expect(model.statistics?.mean == 2.5)
        #expect(model.values?.distinctCount == 4)
        #expect(model.distribution?.counts.count == 2)

        await model.load(document: doc, version: 0, column: 0, expanded: 0, bins: 3)
        #expect(model.distribution == nil)
        #expect(model.overview != nil && model.statistics != nil && model.values != nil)
        await model.load(document: doc, version: 0, column: 0, expanded: all, bins: 3)
        #expect(model.distribution?.counts.count == 3)
        await model.load(document: doc, version: 0, column: 0, expanded: 0, bins: 3, scale: .log10)
        #expect(model.distribution == nil)
        #expect(model.overview != nil && model.statistics != nil && model.values != nil)

        await model.load(document: doc, version: 0, column: 1, expanded: all, bins: 2)
        #expect(model.statistics == nil && model.distribution == nil)
        #expect(model.lengths?.histogram.counts.count == 2)
        await model.load(document: doc, version: 0, column: 1, expanded: 0, bins: 2, scale: .log10)
        #expect(model.lengths != nil, "Numeric scale does not invalidate length statistics")
        await model.load(document: doc, version: 0, column: 1, expanded: 0, bins: 3, scale: .log10)
        #expect(model.lengths == nil)
        #expect(model.overview != nil && model.values != nil)

        let replacement = try document("number\n100\n")
        await model.load(document: replacement, version: 0, column: 0, expanded: AnalysisSection.statistics.mask)
        #expect(model.statistics?.mean == 100)
        #expect(model.overview == nil && model.values == nil && model.lengths == nil)
    }

    @Test
    @MainActor
    static func cancelledTaskDoesNotPublishAndAllowsReplacement() async throws {
        let doc = try document("first,second\n1,100\n2,200\n")
        let model = InspectorModel()
        var task: Task<Void, Never>?
        let observation = model.$loading.sink { section in
            if section == .statistics { task?.cancel() }
        }
        task = Task {
            await model.load(document: doc, version: 0, column: 0, expanded: AnalysisSection.statistics.mask)
        }
        await task?.value
        observation.cancel()
        #expect(model.statistics == nil && model.loading == nil && model.errors.isEmpty)
        await model.load(document: doc, version: 0, column: 1, expanded: AnalysisSection.statistics.mask)
        #expect(model.statistics?.mean == 150)
        #expect(model.matches(document: doc, version: 0, column: 1))
    }

    @Test
    @MainActor
    static func staleRequestsAreSilentAndDoNotRetainPriorResults() async throws {
        let doc = try document("number\n1\n2\n")
        let model = InspectorModel()
        await model.load(document: doc, version: 0, column: 0, expanded: AnalysisSection.statistics.mask)
        let current = try doc.prepareView(spec: QuerySpec(columns: [ColSpec(column: 0, sort: nil, filter: nil)])).run()
        await model.load(document: doc, version: current.version, column: 0, expanded: AnalysisSection.overview.mask)
        #expect(model.statistics == nil && model.overview?.rows == 2)
        await model.load(document: doc, version: 0, column: 0, expanded: AnalysisSection.statistics.mask)
        #expect(model.statistics == nil && model.overview == nil)
        #expect(model.loading == nil && model.errors.isEmpty && model.error == nil)
    }

    @Test
    static func taskCancellationReachesRustAndSuppressesCompletedOutput() async throws {
        let doc = try document("number\n1\n2\n")
        let signal = Cancellation()
        let started = AsyncStream<Void>.makeStream()
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        let task = Task {
            try await runAnalysis(cancellation: signal) {
                let result = try doc.analyzeOverview(version: 0, column: 0, cancellation: signal)
                // Hold a successful result until the owning Task has been cancelled.
                started.continuation.yield(())
                started.continuation.finish()
                release.wait()
                return result
            }
        }
        for await _ in started.stream { break }
        task.cancel()
        release.signal()
        do {
            _ = try await task.value
            Issue.record("Cancelled Task must not publish a successful background result")
        } catch is CancellationError {}
        do {
            _ = try doc.analyzeOverview(version: 0, column: 0, cancellation: signal)
            Issue.record("Task cancellation must reach the Rust signal")
        } catch GewuError.Cancelled {}
        #expect(try doc.analyzeOverview(version: 0, column: 0, cancellation: Cancellation()).rows == 2)
    }

    @Test
    static func alreadyCancelledTaskDoesNotDispatchWork() async throws {
        let signal = Cancellation()
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await runAnalysis(cancellation: signal) {
                Issue.record("Already cancelled work must not be dispatched")
                return 1
            }
        }
        do {
            _ = try await task.value
            Issue.record("Expected task cancellation")
        } catch is CancellationError {}
    }
}
