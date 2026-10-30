import SwiftUI

struct ChoiceFilterView: View {
    let column: UInt32
    let values: [DistinctValue]
    let onApply: ([Filter]) -> Void

    @State private var query = ""
    @State private var checked: Set<UInt64>

    init(column: UInt32, values: [DistinctValue], current: [Filter], onApply: @escaping ([Filter]) -> Void) {
        self.column = column
        self.values = values
        self.onApply = onApply
        var checked = Set(values.map(\.key))
        for filter in current {
            switch filter {
            case .inValues(_, let keys): checked.formIntersection(keys)
            case .isNull(_, let negate):
                checked.formIntersection(values.filter { ($0.label == nil) != negate }.map(\.key))
            default: break
            }
        }
        _checked = State(initialValue: checked)
    }

    private var shown: [DistinctValue] {
        values.filter { query.isEmpty || ($0.label ?? "(null)").localizedCaseInsensitiveContains(query) }
    }
    var body: some View {
        let mostCount = max(values.lazy.map(\.count).max() ?? 0, 1)
        // Read here so selection changes invalidate the list, not only its deferred bindings.
        let selectedKeys = checked
        VStack(alignment: .leading, spacing: 10) {
            TextField("Find value", text: $query).textFieldStyle(.roundedBorder)
            List {
                ForEach(shown, id: \.key) { value in
                    HStack {
                        Toggle(
                            value.label ?? "(null)",
                            isOn: Binding(
                                get: { selectedKeys.contains(value.key) },
                                set: { selected in
                                    if selected { checked.insert(value.key) } else { checked.remove(value.key) }
                                }
                            )
                        )
                        .toggleStyle(.checkbox)
                        .lineLimit(1)
                        Spacer(minLength: 6)
                        Text(formatCount(value.count)).monospacedDigit().foregroundStyle(.secondary)
                        ProgressView(value: Double(value.count), total: Double(mostCount)).frame(width: 42)
                    }
                    .help(value.label ?? "(null)")
                }
            }
            .listStyle(.bordered)
            .frame(height: 230)
            Text("\(formatCount(UInt64(values.count))) values").font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Select All") { checked.formUnion(shown.map(\.key)) }
                Button("Clear") { checked.subtract(shown.map(\.key)) }
                Spacer()
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
        }
    }

    private func apply() {
        let everything = Set(values.map(\.key)).isSubset(of: checked)
        onApply(everything ? [] : [.inValues(column: column, keys: checked.sorted())])
    }
}
