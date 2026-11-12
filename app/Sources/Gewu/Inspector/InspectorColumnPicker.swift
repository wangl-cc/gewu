import SwiftUI

/// The same native column chooser for analysis and chart axes.
struct InspectorColumnPicker: NSViewRepresentable {
    let label: String
    let columns: [(Int, String)]
    var emptySelectionTitle: String? = nil
    @Binding var selection: Int?

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
        var items = columns
        if emptySelectionTitle != nil || selection == nil {
            items.insert((-1, emptySelectionTitle ?? "Select column"), at: 0)
        }
        if button.itemArray.map({ $0.tag }) != items.map({ $0.0 })
            || button.itemTitles != items.map({ $0.1 })
        {
            button.removeAllItems()
            for (column, name) in items {
                let item = NSMenuItem(title: name, action: nil, keyEquivalent: "")
                item.tag = column
                button.menu?.addItem(item)
            }
        }
        button.selectItem(withTag: selection ?? -1)
        button.setAccessibilityLabel(label)
        button.toolTip = button.titleOfSelectedItem
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSPopUpButton, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? nsView.intrinsicContentSize.width, height: nsView.intrinsicContentSize.height)
    }

    final class Coordinator: NSObject {
        var selection: Binding<Int?>
        init(selection: Binding<Int?>) { self.selection = selection }

        @objc func selectColumn(_ sender: NSPopUpButton) {
            guard let column = sender.selectedItem?.tag else { return }
            selection.wrappedValue = column < 0 ? nil : column
        }
    }
}
