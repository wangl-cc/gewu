import GewuKit
import SwiftUI

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
