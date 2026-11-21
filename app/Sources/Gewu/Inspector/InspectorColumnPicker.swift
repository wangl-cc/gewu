import GewuKit
import SwiftUI

/// The same native column chooser for analysis and chart axes.
struct InspectorColumnPicker: NSViewRepresentable {
    let label: String
    let columns: [(ColumnID, String)]
    var emptySelectionTitle: String? = nil
    @Binding var selection: ColumnID?

    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }

    func makeNSView(context: Context) -> NSPopUpButton {
        let button = NSPopUpButton(frame: .zero, pullsDown: false)
        button.controlSize = .small
        button.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        button.cell?.lineBreakMode = .byTruncatingMiddle
        button.setContentHuggingPriority(.defaultLow, for: .horizontal)
        button.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        button.target = context.coordinator
        button.action = #selector(Coordinator.selectColumn(_:))
        return button
    }

    func updateNSView(_ button: NSPopUpButton, context: Context) {
        context.coordinator.selection = $selection
        var items: [(ColumnID?, String)] = columns.map { ($0.0, $0.1) }
        if emptySelectionTitle != nil || selection == nil {
            items.insert((nil, emptySelectionTitle ?? "Select column"), at: 0)
        }
        if context.coordinator.items != items.map(\.0) || button.itemTitles != items.map(\.1) {
            button.removeAllItems()
            for (_, name) in items {
                // Use NSMenuItem directly so duplicate titles or names matching the empty title are preserved.
                let item = NSMenuItem(title: name, action: nil, keyEquivalent: "")
                button.menu?.addItem(item)
            }
            context.coordinator.items = items.map(\.0)
        }
        let index = context.coordinator.items.firstIndex(of: selection) ?? 0
        if index < button.numberOfItems {
            button.selectItem(at: index)
        }
        button.setAccessibilityLabel(label)
        button.toolTip = button.titleOfSelectedItem
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSPopUpButton, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? nsView.intrinsicContentSize.width, height: nsView.intrinsicContentSize.height)
    }

    @MainActor
    final class Coordinator: NSObject {
        var selection: Binding<ColumnID?>
        var items: [ColumnID?] = []
        init(selection: Binding<ColumnID?>) { self.selection = selection }

        @objc func selectColumn(_ sender: NSPopUpButton) {
            let index = sender.indexOfSelectedItem
            guard items.indices.contains(index) else { return }
            selection.wrappedValue = items[index]
        }
    }
}
