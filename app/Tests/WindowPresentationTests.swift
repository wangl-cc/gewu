import AppKit
import Foundation
import Testing

@Suite @MainActor
struct WindowPresentationTests {
    @Test func newFiltersPreferBoundedChoicesBeforeTypeSpecificEditors() throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("filter-modes-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let rows = (0..<1200).map { "\($0 % 3),group\($0 % 3),\($0),item\($0)" }
        try (["few_numbers,few_labels,many_numbers,many_labels"] + rows).joined(separator: "\n")
            .write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let view = dataset.view()
        let expected: [FilterMode] = [.exact, .exact, .range, .contains]
        for (index, column) in dataset.info().columns.enumerated() {
            #expect(FilterMode.initial(capabilities: column.capabilities, current: nil) == nil)
            let choices = try view.filterChoices(column: UInt32(index), cancellation: Cancellation())
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
                current: Filter(condition: .contains(text: "group", caseSensitive: false), includeNulls: true))
                == .contains)
        #expect(
            FilterMode.initial(
                capabilities: text,
                current: Filter(condition: .exact(values: ["group1"], exclude: false), includeNulls: true))
                == .exact)
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
