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
        let view = try Self.dataset().view()
        let viewer = Viewer(view: view, spec: Self.spec(city: nil, district: nil))
        let older = Task { await viewer.apply(Self.spec(city: "bj", district: nil), debounce: true) }
        // Let the older request start first; the main actor runs tasks in order.
        await Task.yield()
        let newer = await viewer.apply(Self.spec(city: "sh", district: nil))
        guard case .applied = newer else { Issue.record("The newest request must apply"); return }
        guard case .superseded = await older.value else { Issue.record("The older request must not apply"); return }
        #expect(viewer.applied.spec == Self.spec(city: "sh", district: nil))
        #expect(viewer.applied.view.rowCount() == 2)
    }

    @Test func failedApplyKeepsTheAppliedView() async throws {
        let initial = try Self.dataset().view()
        let viewer = Viewer(view: initial, spec: Self.spec(city: nil, district: nil))
        var invalid = Self.spec(city: nil, district: nil)
        invalid.columns[0].filter = Filter(condition: .range(min: "1", max: nil), includeNulls: false)
        guard case .failed = await viewer.apply(invalid) else { Issue.record("A text range must fail"); return }
        #expect(viewer.applied.view === initial)
    }

    @Test func resultsAreSharedWithinAViewAndReleasedWhenItChanges() async throws {
        let view = try Self.dataset().view()
        let viewer = Viewer(view: view, spec: Self.spec(city: nil, district: nil))
        _ = await viewer.apply(Self.spec(city: "bj", district: "haidian"))
        let facet = try await viewer.facet(1)
        #expect(try await viewer.facet(1) === facet, "One computation per View and column")
        #expect(facet.rowCount() == 2, "The district facet keeps city = bj only")

        // Column order does not affect any cached result.
        var reordered = viewer.applied.spec
        reordered.columns.reverse()
        try viewer.reorder([1, 0], spec: reordered)
        #expect(try await viewer.facet(1) === facet)

        _ = await viewer.apply(Self.spec(city: "sh", district: "pudong"))
        let replaced = try await viewer.facet(1)
        #expect(replaced !== facet)
        let labels = try await viewer.filterChoices(1).values?.map(\.label)
        #expect(labels == ["pudong", "xuhui"])
    }

    @Test func closingSupersedesPendingQueries() async throws {
        let view = try Self.dataset().view()
        let viewer = Viewer(view: view, spec: Self.spec(city: nil, district: nil))
        let pending = Task { await viewer.apply(Self.spec(city: "bj", district: nil), debounce: true) }
        await Task.yield()
        viewer.close()
        guard case .superseded = await pending.value else { Issue.record("Closing must stop queries"); return }
        #expect(viewer.applied.spec == Self.spec(city: nil, district: nil))
    }
}
