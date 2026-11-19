import Foundation
import Observation
import Testing

@Suite
struct FindModelTests {
    private static let c0 = ColumnId(raw: 0)
    private static let c1 = ColumnId(raw: 1)

    @Test @MainActor
    func emptyInvalidationDoesNotPublishDisplayChanges() {
        let find = FindModel()
        var publications = 0
        withObservationTracking {
            _ = (find.query, find.currentMatch, find.isFinding, find.hasMatches)
            _ = (find.revision, find.highlightRevision, find.status)
        } onChange: {
            publications += 1
        }
        find.invalidate()
        find.stop()
        #expect(publications == 0)
        #expect(find.revision == 0 && find.highlightRevision == 0)
    }

    @Test @MainActor
    func delayedReplacementNavigationAndHighlightsUseOneCapturedFind() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-find-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nold,120\nnew,12\nnew,12\n".write(to: path, atomically: true, encoding: .utf8)
        let document = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: document)
        let originalView = model.ordered

        model.find.query = "old"
        model.updateFind(debounce: true)
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.hasMatches != nil }

        #expect(model.find.currentMatch?.displayRow == 1)
        #expect(model.find.currentMatch?.column == Self.c1)
        #expect(model.ordered === originalView)
        #expect(model.ordered.rowCount() == 3)

        // Only the requested on-screen column is evaluated and cached.
        model.find.requestHighlight(rows: 0..<3, columns: [Self.c1])
        try await waitUntil { model.find.hasHighlights }
        #expect((0..<3).map { model.find.isMatch(row: $0, column: Self.c1) } == [false, true, true])
        #expect(!(0..<3).contains { model.find.isMatch(row: $0, column: Self.c0) })
        let revision = model.find.highlightRevision
        model.find.requestHighlight(rows: 1..<3, columns: [Self.c1])
        #expect(model.find.highlightRevision == revision, "Cached blocks are not requested again")
        model.find.requestHighlight(rows: 0..<3, columns: [Self.c1, Self.c0])
        try await waitUntil { model.find.highlightRevision != revision }
        #expect(!(0..<3).contains { model.find.isMatch(row: $0, column: Self.c0) })

        model.findNext()
        try await waitUntil { model.find.currentMatch?.displayRow == 2 }
        model.findNext()
        try await waitUntil { model.find.currentMatch?.displayRow == 1 }
        model.findNext(backwards: true)
        try await waitUntil { model.find.currentMatch?.displayRow == 2 }

        model.find.query = "old"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.column == Self.c0 }
        #expect(model.ordered === originalView)
        model.find.query = ""
        model.updateFind()
        #expect(model.find.currentMatch == nil && !model.find.hasHighlights)
        #expect(model.find.canNavigate == false)
        model.stop()
    }

    @Test @MainActor
    func viewRefreshDefersPreparationAndRestoresLatestFind() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-refresh-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nold,120\nnew,12\nnew,12\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }

        model.setSortDirection(column: Self.c1, descending: true)
        #expect(model.isBusy)
        model.find.query = "120"
        model.updateFind()
        #expect(!model.find.canNavigate)
        try await waitUntil { !model.isBusy && model.find.hasMatches == true }
        #expect(model.errorMessage == nil)
        #expect(model.find.currentMatch?.displayRow == 0)
        #expect(model.find.currentMatch?.column == Self.c1)
        #expect(model.ordered.rowCount() == 3)
        model.stop()
    }

    @Test @MainActor
    func highlightRequestsDoNotPublishSynchronouslyOrClearReplacementQueries() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-highlight-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value\nnew\nold\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        model.find.query = "new"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }

        let beforeRequest = model.find.highlightRevision
        model.find.requestHighlight(rows: 0..<1, columns: [ColumnId(raw: 0)])
        #expect(model.find.highlightRevision == beforeRequest)
        model.find.query = "old"
        model.updateFind()
        let replacementRevision = model.find.highlightRevision
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.displayRow == 1 }
        #expect(model.find.highlightRevision == replacementRevision)
        #expect(!model.find.hasHighlights)
        model.stop()
    }

    @Test @MainActor
    func burstNavigationConsumesEveryStepWithinOnePage() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-burst-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value\nnew\nnew\nnew\nnew\nnew\nnew\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        model.find.query = "new"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.displayRow == 0 }

        // The first page holds every match, so presses must not be coalesced.
        model.findNext()
        model.findNext()
        model.findNext(backwards: true)
        #expect(model.find.currentMatch?.displayRow == 1)
        model.findNext()
        model.findNext()
        #expect(model.find.currentMatch?.displayRow == 3)
        model.stop()
    }

    @Test @MainActor
    func navigationCrossesAFetchedPageWithoutLosingSteps() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-page-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let rows = (0..<40).map { _ in "new" }.joined(separator: "\n")
        try "value\n\(rows)\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        model.find.query = "new"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.displayRow == 0 }

        // The first page covers 32 matches; 35 steps need one more fetch.
        for _ in 0..<35 { model.findNext() }
        try await waitUntil { !model.find.isFinding && model.find.currentMatch?.displayRow == 35 }
        model.stop()
    }

    @Test @MainActor
    func columnReorderPreservesTheCurrentMatch() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-reorder-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nold,120\nnew,12\nnew,12\n".write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let model = DocumentModel(url: path, dataset: dataset)
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.currentMatch?.displayRow == 1 }
        model.findNext()
        #expect(model.find.currentMatch?.displayRow == 2)

        model.reorderColumns([Self.c1, Self.c0])
        #expect(model.find.currentMatch?.displayRow == 2)
        #expect(model.find.currentMatch?.column == Self.c1)
        #expect(!model.find.isFinding)
        #expect(model.errorMessage == nil)
        model.stop()
    }

    @MainActor
    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("Timed out waiting for Find state")
    }
}
