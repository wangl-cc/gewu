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
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Picker("Match", selection: $draft.mode) {
                    Text("Contains").tag(TextMatch.contains)
                    Text("Equals").tag(TextMatch.equals)
                }.labelsHidden().fixedSize()
                TextField("Enter a value", text: $draft.text)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityLabel("Filter text")
            }
            Toggle("Match case", isOn: $draft.caseSensitive)
                .toggleStyle(.checkbox).font(.caption).foregroundStyle(.secondary)
        }
    }
}
