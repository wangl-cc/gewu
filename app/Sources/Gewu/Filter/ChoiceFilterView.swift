import GewuKit
import SwiftUI

struct ChoiceFilterView: View {
    let values: [ValueChoice]
    let nullCount: UInt64
    @Binding var draft: FilterDraft
    @State private var query = ""
    var body: some View {
        let selected = draft.exact
        VStack(alignment: .leading, spacing: 10) {
            TextField("Find value", text: $query).textFieldStyle(.roundedBorder)
            HStack(spacing: 8) {
                Button("Select All") { draft.selectAllValues(true) }
                    .help("Select every value and missing values, including those hidden by search")
                Button("Deselect All") { draft.selectAllValues(false) }
                    .help("Deselect every value and missing values, including those hidden by search")
                Spacer()
            }.controlSize(.small).buttonStyle(.bordered)
            List {
                Toggle(isOn: $draft.includesMissing) {
                    HStack(spacing: 6) {
                        Image(systemName: "circle.dashed").foregroundStyle(.secondary)
                        Text("Missing").italic()
                        Text("null").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Text(formatCount(nullCount)).monospacedDigit().foregroundStyle(.secondary)
                    }
                }.toggleStyle(.checkbox).help("Missing values, not the text ‘Missing’")
                ForEach(
                    values.filter { query.isEmpty || $0.label.localizedCaseInsensitiveContains(query) },
                    id: \.exactValue
                ) { item in
                    Toggle(
                        isOn: Binding(
                            get: { selected.contains(item.exactValue) },
                            set: { draft.setValue(item.exactValue, selected: $0) })
                    ) {
                        HStack {
                            Text(item.label.isEmpty ? "(empty string)" : item.label).lineLimit(1)
                            Spacer()
                            Text(formatCount(item.count)).monospacedDigit().foregroundStyle(.secondary)
                        }
                    }.toggleStyle(.checkbox).help(item.label)
                }
            }.listStyle(.plain).scrollContentBackground(.hidden)
                .frame(height: min(240, CGFloat(values.count + 1) * 28))
        }
    }
}
