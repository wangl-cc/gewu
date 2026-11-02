import SwiftUI

struct ChoiceFilterView: View {
    let values: [ValueChoice]
    @Binding var draft: ExactFilterDraft
    @State private var query = ""
    var body: some View {
        let selected = draft
        VStack {
            TextField("Find value", text: $query).textFieldStyle(.roundedBorder)
            List {
                ForEach(
                    values.filter { query.isEmpty || $0.label.localizedCaseInsensitiveContains(query) },
                    id: \.literalIdentity
                ) { item in
                    Toggle(
                        isOn: Binding(
                            get: { selected.contains(item.literalIdentity) },
                            set: { draft.set(item.literalIdentity, selected: $0) })
                    ) {
                        HStack {
                            Text(item.label.isEmpty ? "(empty string)" : item.label).lineLimit(1)
                            Spacer()
                            Text(formatCount(item.count)).monospacedDigit().foregroundStyle(.secondary)
                        }
                    }.toggleStyle(.checkbox).help(item.label)
                }
            }.listStyle(.bordered).frame(height: 230)
        }
    }
}

extension ValueChoice {
    var literalIdentity: LiteralTextValue { LiteralTextValue(value: value) }
}
