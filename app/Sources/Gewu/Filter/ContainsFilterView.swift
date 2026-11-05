import SwiftUI

struct TextMatchDraft {
    var text = ""
    var mode = TextMatch.contains
    var caseSensitive = false
    init(_ filter: Filter?) {
        if let filter, case .text(let text, let mode, let caseSensitive) = filter.condition {
            self.text = text
            self.mode = mode
            self.caseSensitive = caseSensitive
        }
    }
    var condition: FilterCondition? {
        text.isEmpty ? nil : .text(text: text, mode: mode, caseSensitive: caseSensitive)
    }
}

struct TextMatchEditor: View {
    @Binding var draft: TextMatchDraft
    var body: some View {
        Picker("Match", selection: $draft.mode) {
            Text("Contains").tag(TextMatch.contains)
            Text("Equals").tag(TextMatch.equals)
        }.pickerStyle(.segmented).labelsHidden()
        TextField(draft.mode == .equals ? "Equals…" : "Contains…", text: $draft.text)
            .textFieldStyle(.roundedBorder)
        Toggle("Match Case", isOn: $draft.caseSensitive).toggleStyle(.checkbox)
    }
}
