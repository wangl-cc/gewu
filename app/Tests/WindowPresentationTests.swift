import AppKit
import Foundation
import SwiftUI
import Testing

@Suite @MainActor
struct WindowPresentationTests {
    @Test func filterDescriptionsKeepNullLiteralAndExactBoundsDistinct() {
        let missing = Filter(condition: .exact(values: [], exclude: false), includeNulls: true)
        let literal = Filter(condition: .exact(values: ["Missing"], exclude: false), includeNulls: false)
        #expect(missing.conditionSummary == "Only missing")
        #expect(literal.conditionSummary == "= “Missing”")
        #expect(
            Filter(condition: .exact(values: [""], exclude: false), includeNulls: false).conditionSummary
                == "= (empty string)")
        let range = Filter(condition: .range(min: "9007199254740993", max: nil), includeNulls: true)
        #expect(range.conditionSummary == "≥ 9007199254740993 · + missing")
        let text = Filter(
            condition: .text(text: "line\nbreak", mode: .equals, caseSensitive: true), includeNulls: false)
        #expect(!text.conditionSummary.contains("\n"))
        #expect(text.detailSummary.contains("Case sensitive"))
    }

    @Test func cellPreviewHasBoundedLayoutForShortAndLongValues() {
        for text in ["short value", String(repeating: "a long value that wraps\n", count: 100)] {
            let host = NSHostingController(rootView: CellPreview(name: "value", text: text))
            let size = host.view.fittingSize
            #expect(size.width == 440)
            #expect(size.height > 0 && size.height < 400)
            host.view.frame.size = size
            host.view.layoutSubtreeIfNeeded()
        }
    }

    @Test func filterFacetIgnoresOnlyTheEditedColumnsFilter() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("filter-facet-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "city,district\nbj,haidian\nbj,chaoyang\nsh,pudong\nsh,xuhui\n"
            .write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        func exact(_ value: String) -> Filter {
            Filter(condition: .exact(values: [value], exclude: false), includeNulls: false)
        }
        let c0 = ColumnId(raw: 0)
        let c1 = ColumnId(raw: 1)
        let filters = [
            ColumnFilter(column: c0, filter: exact("bj")),
            ColumnFilter(column: c1, filter: exact("haidian")),
        ]
        let filtered = try dataset.filter(filters: filters)
        let ordered = try filtered.sort(sorts: [ColumnSort(column: c1, descending: true)])
        #expect(ordered.rowCount() == 1)

        // The district editor keeps city = bj but not its own district = haidian.
        let viewer = Viewer(
            dataset: dataset, filtered: filtered, ordered: ordered, filters: filters,
            sorts: [ColumnSort(column: c1, descending: true)])
        let district = try await viewer.facet(c1)
        let choices = try district.filterChoices(column: c1)
        #expect(choices.values?.map(\.label) == ["chaoyang", "haidian"])

        // A column without its own filter reuses the applied filtered view.
        let cityOnlyFilters = [ColumnFilter(column: c0, filter: exact("bj"))]
        let cityOnlyFiltered = try dataset.filter(filters: cityOnlyFilters)
        let cityOnlyViewer = Viewer(
            dataset: dataset, filtered: cityOnlyFiltered, ordered: cityOnlyFiltered.unsorted(),
            filters: cityOnlyFilters)
        #expect(try await cityOnlyViewer.facet(c1) === cityOnlyFiltered)
    }

    @Test func newFiltersPreferBoundedChoicesBeforeTypeSpecificEditors() throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("filter-modes-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let rows = (0..<1200).map { "\($0 % 3),group\($0 % 3),\($0),item\($0)" }
        try (["few_numbers,few_labels,many_numbers,many_labels"] + rows).joined(separator: "\n")
            .write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let view = dataset.unfiltered()
        let expected: [FilterMode] = [.exact, .exact, .range, .text]
        for (index, column) in dataset.info().columns.enumerated() {
            #expect(FilterMode.initial(capabilities: column.capabilities, current: nil) == nil)
            let choices = try view.filterChoices(column: column.id)
            #expect(
                FilterMode.initial(capabilities: column.capabilities, current: nil, choices: choices)
                    == expected[index])
        }

        let numeric = dataset.info().columns[0].capabilities
        let text = dataset.info().columns[1].capabilities
        #expect(
            FilterMode.initial(
                capabilities: numeric, current: Filter(condition: .range(min: "1", max: "2"), includeNulls: false))
                == .range)
        #expect(
            FilterMode.initial(
                capabilities: text,
                current: Filter(
                    condition: .text(text: "group", mode: .equals, caseSensitive: false), includeNulls: true))
                == .text)
        // Value selection waits for choices and needs a complete list of values.
        let selected = Filter(condition: .exact(values: ["group1"], exclude: false), includeNulls: true)
        #expect(FilterMode.initial(capabilities: text, current: selected) == nil)
        let labels = try view.filterChoices(column: ColumnId(raw: 1))
        #expect(FilterMode.initial(capabilities: text, current: selected, choices: labels) == .exact)
        let manyLabels = dataset.info().columns[3].capabilities
        let unlisted = try view.filterChoices(column: ColumnId(raw: 3))
        #expect(unlisted.values == nil)
        #expect(FilterMode.initial(capabilities: manyLabels, current: selected, choices: unlisted) == .text)
    }

    @Test func inspectorChoicesSurviveTypeChangesWithoutRequestingUnsupportedLogBins() {
        let settings = InspectorSettings()
        #expect(settings.expanded(distribution: true) == 7)
        #expect(settings.expanded(distribution: false) == 9)
        settings.histogramBins = 100
        settings.histogramScale = .log10
        settings.frequencyScale = .log10
        settings.setExpanded(.distribution, false)
        settings.setExpanded(.values, true)
        #expect(settings.binningScale(logarithmic: false) == .linear)
        #expect(settings.expanded(distribution: false) & InspectorSection.values.mask != 0)
        #expect(settings.binningScale(logarithmic: true) == .log10)
        #expect(settings.expanded(distribution: true) & InspectorSection.distribution.mask == 0)
        #expect(settings.expanded(distribution: true) & InspectorSection.values.mask != 0)
        #expect(settings.histogramBins == 100 && settings.frequencyScale == .log10)
    }

    @Test func reusedFindCellsKeepSelectedTextLegibleAndClearOldHighlight() throws {
        let cell = MatchCellView()
        let field = NSTextField(labelWithString: "")
        cell.addSubview(field)
        cell.textField = field
        cell.configure(text: "x hit", font: .systemFont(ofSize: 12), alignment: .left, query: "hit", current: true)
        cell.backgroundStyle = .emphasized
        #expect(cell.currentMatch)
        let text = field.attributedStringValue
        #expect(
            text.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor
                == .alternateSelectedControlTextColor)
        #expect(text.attribute(.foregroundColor, at: 2, effectiveRange: nil) as? NSColor == .black)
        #expect(text.attribute(.backgroundColor, at: 2, effectiveRange: nil) as? NSColor == .findHighlightColor)
        cell.backgroundStyle = .normal
        #expect(
            field.attributedStringValue.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor
                == .labelColor)
        cell.configure(text: "null", font: .systemFont(ofSize: 12), alignment: .right, muted: true)
        #expect(!cell.currentMatch && !cell.wholeCellMatch)
        #expect(field.attributedStringValue.attribute(.backgroundColor, at: 0, effectiveRange: nil) == nil)
        #expect(
            field.attributedStringValue.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor
                == .tertiaryLabelColor)
    }

    @Test func emptyHistogramGeometryAndReadOnlyChartsIgnoreMouseGestures() throws {
        let chart = HistogramView(frame: .zero)
        chart.setData(counts: [1, 2], lo: 0, hi: 2, integer: true)
        chart.setSelection(0, 1, snapped: true)
        let down = try #require(
            NSEvent.mouseEvent(
                with: .leftMouseDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
                eventNumber: 1, clickCount: 2, pressure: 1))
        let up = try #require(
            NSEvent.mouseEvent(
                with: .leftMouseUp, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
                eventNumber: 2, clickCount: 2, pressure: 0))
        chart.mouseDown(with: down)
        chart.mouseDragged(with: down)
        chart.mouseUp(with: up)
        #expect(chart.selection.0 == 0 && chart.selection.1 == 1)
        chart.frame.size = NSSize(width: 400, height: 150)
        chart.isInteractive = false
        chart.mouseDown(with: down)
        chart.mouseUp(with: up)
        #expect(chart.selection.0 == 0 && chart.selection.1 == 1)
    }

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

    @Test func inspectorColumnPickerPreservesDuplicateAndSentinelColumnTitles() throws {
        let c0 = ColumnId(raw: 0)
        let c1 = ColumnId(raw: 1)
        let c2 = ColumnId(raw: 2)
        var selection: ColumnId?
        let binding = Binding<ColumnId?>(get: { selection }, set: { selection = $0 })
        let host = NSHostingController(
            rootView: InspectorColumnPicker(
                label: "X axis",
                columns: [(c0, "dup"), (c1, "Source row"), (c2, "dup")],
                emptySelectionTitle: "Source row",
                selection: binding
            ))
        host.view.frame = NSRect(x: 0, y: 0, width: 240, height: 28)
        host.view.layoutSubtreeIfNeeded()

        func findPopUp(_ view: NSView) -> NSPopUpButton? {
            if let button = view as? NSPopUpButton { return button }
            for subview in view.subviews {
                if let found = findPopUp(subview) { return found }
            }
            return nil
        }

        let button = try #require(findPopUp(host.view))
        #expect(button.numberOfItems == 4)
        #expect(button.itemTitles == ["Source row", "dup", "Source row", "dup"])

        let action = try #require(button.action)
        button.selectItem(at: 3)
        _ = button.target?.perform(action, with: button)
        #expect(selection == c2)

        button.selectItem(at: 2)
        _ = button.target?.perform(action, with: button)
        #expect(selection == c1)

        button.selectItem(at: 0)
        _ = button.target?.perform(action, with: button)
        #expect(selection == nil)
    }
}
