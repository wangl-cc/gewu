import Foundation

// Compile with freshly generated gewu.swift and the pure InspectorSection projection.
@main
enum ViewSmoke {
    static func check(_ condition: Bool) {
        precondition(condition)
    }

    static func main() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("gewu-view-smoke-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("source.csv")
        try "id,label\n1,one\n2,two\n3,three\n".write(to: source, atomically: true, encoding: .utf8)
        var dataset: GewuDataset? = try GewuDataset.open(path: source.path)
        let original = dataset!.view()
        let full = QuerySpec(columns: [
            ColSpec(column: 0, sort: nil, filter: nil),
            ColSpec(column: 1, sort: nil, filter: nil),
        ])
        var spec = full
        spec.columns[0].filter = Filter(
            condition: .range(min: "not a number", max: nil), includeNulls: false)
        do {
            _ = try original.query(spec: spec, cancellation: Cancellation())
            preconditionFailure("invalid filter returned a View")
        } catch GewuError.InvalidArgument(let message) {
            check(!message.isEmpty)
        }
        check(original.rowCount() == 3)
        check(!dataset!.info().columns[0].capabilities.contains)
        check(dataset!.info().columns[1].capabilities.contains)
        spec.columns[0].filter = Filter(condition: .contains(text: "", caseSensitive: false), includeNulls: true)
        do {
            _ = try original.query(spec: spec, cancellation: Cancellation())
            preconditionFailure("non-text Contains returned a View")
        } catch GewuError.InvalidArgument {}
        var textSpec = full
        textSpec.columns[1].filter = Filter(condition: .contains(text: "T", caseSensitive: false), includeNulls: false)
        check(try original.query(spec: textSpec, cancellation: Cancellation()).rowCount() == 2)
        spec.columns[0].filter = Filter(
            condition: .range(min: "2", max: nil), includeNulls: false)
        spec.columns[0].sort = ColSort(priority: 0, descending: true)
        let filtered = try original.query(spec: spec, cancellation: Cancellation())
        check(filtered.rowCount() == 2)
        check(try filtered.rows(start: 0, count: 10, columns: [0]).sourceRows == [2, 1])
        check(try filtered.analyzeStatistics(column: 0, cancellation: Cancellation()).mean == 2.5)
        let numericCapabilities = try dataset!.analysisCapabilities(column: 0)
        check(numericCapabilities.statistics && numericCapabilities.distribution && numericCapabilities.values)
        check(!numericCapabilities.lengths && !numericCapabilities.structure)
        let textCapabilities = try dataset!.analysisCapabilities(column: 1)
        check(!textCapabilities.statistics && !textCapabilities.distribution)
        check(textCapabilities.values && textCapabilities.lengths && !textCapabilities.structure)
        check(InspectorSection.available(for: numericCapabilities) == [.overview, .statistics, .distribution, .values])
        check(InspectorSection.available(for: textCapabilities) == [.overview, .values, .lengths])
        check(
            InspectorSection.available(
                for: AnalysisCapabilities(
                    statistics: false, distribution: false, values: false, lengths: false, structure: false)) == [
                    .overview
                ])
        check(InspectorSection.values.mask == 8)
        dataset = nil

        let reordered = try filtered.reorderColumns(order: [1, 0])
        check(reordered.columns() == [1, 0])
        check(try reordered.rows(start: 0, count: 1, columns: reordered.columns()).cells == ["three", "3"])
        let restored = try filtered.query(spec: full, cancellation: Cancellation())
        check(restored.rowCount() == 3)
        check(try original.rows(start: 0, count: 3, columns: [0]).sourceRows == [0, 1, 2])
        check(try filtered.analyzeOverview(column: 0, cancellation: Cancellation()).rows == 2)

        let cancelled = Cancellation()
        cancelled.cancel()
        do {
            _ = try filtered.query(spec: full, cancellation: cancelled)
            preconditionFailure("cancelled query returned a View")
        } catch GewuError.Cancelled {}
        do {
            _ = try filtered.analyzeOverview(column: 0, cancellation: cancelled)
            preconditionFailure("cancelled analysis returned a result")
        } catch GewuError.Cancelled {}
        check(try filtered.query(spec: full, cancellation: Cancellation()).rowCount() == 3)

        let choices = try filtered.textChoices(column: 1)
        let page = try choices.page(query: "", after: nil, limit: 10, cancellation: Cancellation())
        check(page.values.map(\.value) == ["three", "two"])
        let find = try filtered.find(query: "three", columns: [1])
        let hits = try find.next(after: nil, limit: 10, cancellation: Cancellation())
        check(hits.count == 1 && hits[0].displayRow == 0)
        let scatter = try filtered.analyzeScatter(
            xColumn: nil, yColumn: 0, limit: 10,
            xScale: .linear, yScale: .linear, cancellation: Cancellation())
        check(scatter.eligibleRows == 2)
        let destination = directory.appendingPathComponent("result.csv")
        let export = try filtered.export(columns: [0], format: .csv)
        try export.write(path: destination.path)
        check(try String(contentsOf: destination, encoding: .utf8) == "id\r\n3\r\n2\r\n")
        print("Dataset/View Swift binding smoke test passed")
    }
}
