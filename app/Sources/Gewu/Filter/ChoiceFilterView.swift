import SwiftUI

/// Arrow text equality compares UTF-8, unlike Swift's canonical-equivalence equality.
struct LiteralTextValue: Hashable {
    let value: String
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.value.utf8.elementsEqual(rhs.value.utf8) }
    func hash(into hasher: inout Hasher) { for byte in value.utf8 { hasher.combine(byte) } }
    var label: String { value.isEmpty ? "(empty string)" : value }
}

struct ExactFilterDraft {
    var exclude = true
    var values: Set<LiteralTextValue> = []
    init(_ filter: Filter?) {
        if let filter, case .exact(let values, let exclude) = filter.condition {
            self.values = Set(values.map { LiteralTextValue(value: $0) })
            self.exclude = exclude
        }
    }
    func contains(_ value: LiteralTextValue) -> Bool { values.contains(value) != exclude }
    mutating func set(_ value: LiteralTextValue, selected: Bool) {
        if selected != exclude { values.insert(value) } else { values.remove(value) }
    }
    mutating func all(_ selected: Bool) {
        exclude = selected
        values.removeAll()
    }
    var condition: FilterCondition {
        .exact(values: values.map(\.value).sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }, exclude: exclude)
    }
}

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
