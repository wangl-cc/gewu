import AppKit
import SwiftUI

/// Native search appearance and editing, connected to the document's Find navigation.
struct FindSearchField: NSViewRepresentable {
    @Binding var text: String
    var focusRequest: Int
    var next: (Bool) -> Void
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
        if field.focusRequest != focusRequest {
            field.focusRequest = focusRequest
            field.needsSearchFocus = true
            field.focusIfRequested()
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Field: NSSearchField {
        var focusRequest = 0
        var needsSearchFocus = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            focusIfRequested()
        }

        func focusIfRequested() {
            guard needsSearchFocus, let window else { return }
            needsSearchFocus = false
            window.makeFirstResponder(self)
        }
    }

    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var parent: FindSearchField

        init(_ parent: FindSearchField) { self.parent = parent }

        @objc func searchChanged(_ sender: NSSearchField) {
            parent.text = sender.stringValue
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
