import SwiftUI

struct ContainsFilterView: View {
    let column: UInt32
    let nullable: Bool
    let onApply: ([Filter]) -> Void
    @State private var draft: TextContainsDraft

    init(column: UInt32, nullable: Bool, current: [Filter], onApply: @escaping ([Filter]) -> Void) {
        self.column = column
        self.nullable = nullable
        self.onApply = onApply
        _draft = State(initialValue: TextContainsDraft(current))
    }

    var body: some View {
        ContainsEditor(column: column, nullable: nullable, draft: $draft, onApply: onApply)
    }
}

/// Both text mode switching and contains-only types use the same editor and draft.
struct ContainsEditor: View {
    let column: UInt32
    let nullable: Bool
    @Binding var draft: TextContainsDraft
    let onApply: ([Filter]) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("Contains…", text: $draft.text).textFieldStyle(.roundedBorder)
                .onSubmit { onApply(draft.filters()) }
            Toggle("Match Case", isOn: $draft.caseSensitive).toggleStyle(.checkbox)
            HStack {
                if nullable { NullModePicker(selection: $draft.nullMode) }
                Spacer()
                Button("Apply") { onApply(draft.filters()) }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }
}
