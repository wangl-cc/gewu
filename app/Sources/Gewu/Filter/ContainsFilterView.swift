import SwiftUI

struct TextContainsDraft {
    var text = ""
    var caseSensitive = false
    init(_ filter: Filter?) {
        if let filter, case .contains(let text, let caseSensitive) = filter.condition {
            self.text = text
            self.caseSensitive = caseSensitive
        }
    }
    var condition: FilterCondition? {
        text.isEmpty ? nil : .contains(text: text, caseSensitive: caseSensitive)
    }
}

struct ContainsEditor: View {
    @Binding var draft: TextContainsDraft
    var body: some View {
        TextField("Contains…", text: $draft.text).textFieldStyle(.roundedBorder)
        Toggle("Match Case", isOn: $draft.caseSensitive).toggleStyle(.checkbox)
    }
}
