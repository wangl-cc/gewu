import Foundation
import Testing

@Suite @MainActor
struct WindowPresentationTests {
    @Test func inspectorDefaultsOpenAndRemembersExplicitClose() throws {
        let name = "gewu-inspector-\(UUID())"
        let preferences = try #require(UserDefaults(suiteName: name))
        defer { preferences.removePersistentDomain(forName: name) }
        let url = URL(fileURLWithPath: "/unused.csv")
        let first = WorkbookModel(url: url, preferences: preferences)
        #expect(first.showsInspector)
        first.showsInspector = false
        let second = WorkbookModel(url: url, preferences: preferences)
        #expect(!second.showsInspector)
        second.showsInspector = true
        #expect(WorkbookModel(url: url, preferences: preferences).showsInspector)
    }

    @Test func filterSummariesDistinguishNullAndEmptyStringSelections() {
        #expect(Filter(condition: .exact(values: [], exclude: false), includeNulls: true).summary == "Only nulls")
        #expect(Filter(condition: .exact(values: [], exclude: true), includeNulls: false).summary == "Not null")
        #expect(
            Filter(condition: .exact(values: [""], exclude: false), includeNulls: false).summary == "“” · hide nulls")
        #expect(
            Filter(condition: .range(min: "9007199254740993", max: nil), includeNulls: false).summary.contains(
                "9007199254740993"))
    }
}
