import AppKit
import SwiftUI

/// Native search appearance and editing, connected to the table's Find.
/// `isFocused` follows the field's keyboard focus; setting it to `true` moves
/// focus to the field.
struct FindSearchField: NSViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    var next: (_ backwards: Bool) -> Void
    var cancel: () -> Void

    func makeNSView(context: Context) -> Field {
        let field = Field()
        field.placeholderString = "Find"
        field.setAccessibilityLabel("Find in visible columns")
        field.toolTip = "Find in visible columns (⌘F). Return: next match; Shift-Return: previous match."
        field.delegate = context.coordinator
        field.target = context.coordinator
        field.action = #selector(Coordinator.searchChanged(_:))
        field.sendsSearchStringImmediately = true
        return field
    }

    func updateNSView(_ field: Field, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
        field.wantsFocus = isFocused
        field.focusIfWanted()
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Field: NSSearchField {
        var wantsFocus = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            focusIfWanted()
        }

        func focusIfWanted() {
            guard wantsFocus, currentEditor() == nil, let window else { return }
            window.makeFirstResponder(self)
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var parent: FindSearchField

        init(_ parent: FindSearchField) { self.parent = parent }

        @objc func searchChanged(_ sender: NSSearchField) {
            parent.text = sender.stringValue
        }

        func controlTextDidBeginEditing(_ notification: Notification) {
            if !parent.isFocused { parent.isFocused = true }
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            if parent.isFocused { parent.isFocused = false }
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            switch commandSelector {
            case #selector(NSResponder.insertNewline(_:)):
                parent.next(NSApp.currentEvent?.modifierFlags.contains(.shift) == true)
            case #selector(NSResponder.cancelOperation(_:)):
                parent.cancel()
            default:
                return false
            }
            return true
        }
    }
}
