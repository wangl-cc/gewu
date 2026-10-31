import Foundation
import Testing

@Suite
struct FindModelTests {
    @Test @MainActor
    func delayedReplacementNavigationAndHighlightsUseOneCapturedFind() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-find-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nold,120\nnew,12\nnew,12\n".write(to: path, atomically: true, encoding: .utf8)
        let document = try GewuDocument.open(path: path.path)
        let model = DocumentModel(url: path, document: document)
        let originalView = model.current

        model.find.query = "old"
        model.updateFind(debounce: true)
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.hasMatches != nil }

        #expect(model.find.currentMatch?.displayRow == 1)
        #expect(model.find.currentMatch?.column == 1)
        #expect(model.current == originalView)
        #expect(model.current.rowCount == 3)

        model.find.requestHighlight(start: 0, count: 3)
        try await waitUntil { model.find.highlight != nil }
        #expect(model.find.highlight?.matched == [false, false, false, true, false, true])
        #expect(model.find.isMatch(row: 1, column: 1))
        #expect(!model.find.isMatch(row: 1, column: 0))
        #expect(!model.find.isMatch(row: 0, column: 1))

        model.findNext()
        try await waitUntil { model.find.currentMatch?.displayRow == 2 }
        model.findNext()
        try await waitUntil { model.find.currentMatch?.displayRow == 1 }
        model.findNext(backwards: true)
        try await waitUntil { model.find.currentMatch?.displayRow == 2 }

        model.find.query = "old"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.column == 0 }
        #expect(model.current == originalView)
        model.find.query = ""
        model.updateFind()
        #expect(model.find.currentMatch == nil && model.find.highlight == nil)
        #expect(model.find.canNavigate == false)
        model.stop()
    }

    @Test @MainActor
    func viewRefreshDefersPreparationAndRestoresLatestFind() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-refresh-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value,number\nold,120\nnew,12\nnew,12\n".write(to: path, atomically: true, encoding: .utf8)
        let model = DocumentModel(url: path, document: try GewuDocument.open(path: path.path))
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }

        model.setSort([SortKey(column: 1, descending: true)])
        #expect(model.isBusy)
        model.find.query = "120"
        model.updateFind()
        #expect(!model.find.canNavigate)
        try await waitUntil { !model.isBusy && model.find.hasMatches == true }
        #expect(model.errorMessage == nil)
        #expect(model.find.currentMatch?.displayRow == 0)
        #expect(model.find.currentMatch?.column == 1)
        #expect(model.current.rowCount == 3)
        model.stop()
    }

    @Test @MainActor
    func highlightRequestsDoNotPublishSynchronouslyOrClearReplacementQueries() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-highlight-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value\nnew\nold\n".write(to: path, atomically: true, encoding: .utf8)
        let model = DocumentModel(url: path, document: try GewuDocument.open(path: path.path))
        model.find.query = "new"
        model.updateFind()
        try await waitUntil { model.find.hasMatches == true }

        let beforeRequest = model.find.highlightRevision
        model.find.requestHighlight(start: 0, count: 1)
        #expect(model.find.highlightRevision == beforeRequest)
        model.find.query = "old"
        model.updateFind()
        let replacementRevision = model.find.highlightRevision
        try await waitUntil { model.find.hasMatches == true && model.find.currentMatch?.displayRow == 1 }
        #expect(model.find.highlightRevision == replacementRevision)
        #expect(model.find.highlight == nil)
        model.stop()
    }

    @Test @MainActor
    func burstNavigationConsumesEveryStepWithinOnePage() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(
            "gewu-find-burst-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "value\nnew\nnew\nnew\nnew\nnew\nnew\n".write(to: path, atomically: true, encoding: .utf8)
        let model = DocumentModel(url: path, document: try GewuDocument.open(path: path.path))
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
        let model = DocumentModel(url: path, document: try GewuDocument.open(path: path.path))
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
        let model = DocumentModel(url: path, document: try GewuDocument.open(path: path.path))
        model.find.query = "12"
        model.updateFind()
        try await waitUntil { model.find.currentMatch?.displayRow == 1 }
        model.findNext()
        #expect(model.find.currentMatch?.displayRow == 2)

        model.findColumnsChanged()
        #expect(model.find.currentMatch?.displayRow == 2)
        #expect(model.find.currentMatch?.column == 1)
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
