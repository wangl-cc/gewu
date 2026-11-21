import AppKit
import Foundation
import GewuKit
import SwiftUI
import Testing

@Suite @MainActor
struct WindowPresentationTests {
    @Test func filterDescriptionsKeepNullLiteralAndExactBoundsDistinct() {
        let missing: Filter = .onlyNulls
        let literal: Filter = .condition(.exact(values: ["Missing"], exclude: false), includeNulls: false)
        #expect(missing.conditionSummary == "Only missing")
        #expect(literal.conditionSummary == "= “Missing”")
        let empty: Filter = .condition(.exact(values: [""], exclude: false), includeNulls: false)
        #expect(empty.conditionSummary == "= (empty string)")
        let range: Filter = .condition(.range(min: "9007199254740993", max: nil), includeNulls: true)
        #expect(range.conditionSummary == "≥ 9007199254740993 · + missing")
        let text: Filter = .condition(
            .text(text: "line\nbreak", mode: .equals, caseSensitive: true), includeNulls: false)
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

    @Test func newFiltersPreferBoundedChoicesBeforeTypeSpecificEditors() async throws {
        let rows = (0..<1200).map { "\($0 % 3),group\($0 % 3),\($0),item\($0)" }
        let session = try await TestTables.session(
            (["few_numbers,few_labels,many_numbers,many_labels"] + rows).joined(separator: "\n"))
        let view = session.initialRows.rowSet
        let columns = session.source.columns
        let expected: [FilterMode] = [.exact, .exact, .range, .text]
        for (index, column) in columns.enumerated() {
            #expect(FilterMode.initial(capabilities: column.capabilities, current: nil) == nil)
            let choices = try await view.filterChoices(for: column.id)
            #expect(
                FilterMode.initial(capabilities: column.capabilities, current: nil, choices: choices)
                    == expected[index])
        }

        let numeric = columns[0].capabilities
        let text = columns[1].capabilities
        #expect(
            FilterMode.initial(
                capabilities: numeric, current: .condition(.range(min: "1", max: "2"), includeNulls: false))
                == .range)
        #expect(
            FilterMode.initial(
                capabilities: text,
                current: .condition(.text(text: "group", mode: .equals, caseSensitive: false), includeNulls: true))
                == .text)
        // Value selection waits for choices and needs a complete list of values.
        let selected: Filter = .condition(.exact(values: ["group1"], exclude: false), includeNulls: true)
        #expect(FilterMode.initial(capabilities: text, current: selected) == nil)
        let labels = try await view.filterChoices(for: columns[1].id)
        #expect(FilterMode.initial(capabilities: text, current: selected, choices: labels) == .exact)
        let unlisted = try await view.filterChoices(for: columns[3].id)
        #expect(unlisted.values == nil)
        #expect(
            FilterMode.initial(capabilities: columns[3].capabilities, current: selected, choices: unlisted) == .text)
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
        let first = WindowModel(url: url, defaults: preferences)
        #expect(first.showsInspector)
        first.showsInspector = false
        let second = WindowModel(url: url, defaults: preferences)
        #expect(!second.showsInspector)
        second.showsInspector = true
        #expect(WindowModel(url: url, defaults: preferences).showsInspector)
    }

    @Test func inspectorColumnPickerPreservesDuplicateAndSentinelColumnTitles() async throws {
        let columns = try await TestTables.session("a,b,c\n1,2,3\n").source.columns.map(\.id)
        let (c0, c1, c2) = (columns[0], columns[1], columns[2])
        var selection: ColumnID?
        let binding = Binding<ColumnID?>(get: { selection }, set: { selection = $0 })
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
