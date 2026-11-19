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
        let info = dataset!.info()
        let idCol = dataset!.columnId(name: "id")!
        let labelCol = dataset!.columnId(name: "label")!
        check(dataset!.columnId(name: "missing") == nil)
        check(info.columns[0].id == idCol && info.columns[1].id == labelCol)
        let colSet: Set<ColumnId> = [idCol, labelCol]
        check(colSet.contains(idCol))

        let original = dataset!.unfiltered()
        let originalOrdered = original.unsorted()
        do {
            _ = try dataset!.filter(filters: [
                ColumnFilter(
                    column: idCol,
                    filter: Filter(
                        condition: .range(min: "not a number", max: nil),
                        includeNulls: false))
            ])
            preconditionFailure("invalid filter returned a GewuFilteredView")
        } catch GewuError.Filter(_, let message) {
            check(!message.isEmpty)
        }
        check(original.rowCount() == 3)
        check(!info.columns[0].capabilities.textMatch)
        check(info.columns[1].capabilities.textMatch)
        do {
            _ = try dataset!.filter(filters: [
                ColumnFilter(
                    column: idCol,
                    filter: Filter(
                        condition: .text(text: "", mode: .contains, caseSensitive: false),
                        includeNulls: true))
            ])
            preconditionFailure("non-text text match returned a GewuFilteredView")
        } catch GewuError.Internal {}
        let textFiltered = try dataset!.filter(filters: [
            ColumnFilter(
                column: labelCol,
                filter: Filter(
                    condition: .text(text: "T", mode: .contains, caseSensitive: false),
                    includeNulls: false))
        ])
        check(textFiltered.rowCount() == 2)

        let filtered = try dataset!.filter(filters: [
            ColumnFilter(
                column: idCol,
                filter: Filter(
                    condition: .range(min: "2", max: nil),
                    includeNulls: false))
        ])
        let sorted = try filtered.sort(sorts: [ColumnSort(column: idCol, descending: true)])
        check(filtered.rowCount() == 2)
        check(sorted.rowCount() == 2)
        check(try sorted.rows(start: 0, count: 10, columns: [idCol]).sourceRows == [2, 1])
        check(try filtered.statistics(column: idCol).mean == 2.5)
        let numericCapabilities = info.columns[0].analysis
        check(numericCapabilities.statistics && numericCapabilities.distribution && numericCapabilities.values)
        check(!numericCapabilities.lengths && !numericCapabilities.structure)
        let textCapabilities = info.columns[1].analysis
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

        check(try sorted.rows(start: 0, count: 1, columns: [labelCol, idCol]).cells == ["three", "3"])
        let restored = dataset!.unfiltered()
        check(restored.rowCount() == 3)
        check(try originalOrdered.rows(start: 0, count: 3, columns: [idCol]).sourceRows == [0, 1, 2])
        check(try filtered.countNulls(column: idCol) == 0)
        dataset = nil

        let choices = try filtered.filterChoices(column: labelCol)
        check(choices.values?.map(\.value) == ["three", "two"])
        let find = try sorted.find(query: "three", columns: [labelCol])
        let hits = try find.next(after: nil, limit: 10)
        check(hits.count == 1 && hits[0].displayRow == 0 && hits[0].column == labelCol)
        let scatter = try filtered.scatter(
            xColumn: nil, yColumn: idCol, limit: 10,
            xScale: .linear, yScale: .linear)
        check(scatter.eligibleRows == 2)
        let destination = directory.appendingPathComponent("result.csv")
        let export = try sorted.export(columns: [idCol], format: .csv)
        try export.write(path: destination.path)
        check(try String(contentsOf: destination, encoding: .utf8) == "id\r\n3\r\n2\r\n")
        print("Dataset/View Swift binding smoke test passed")
    }
}
