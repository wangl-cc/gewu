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

struct TextFilterView: View {
    let view: GewuView
    let column: UInt32
    @Binding var draft: ExactFilterDraft
    @State private var query = ""
    @State private var after: TextChoiceCursor?
    @State private var revision = 0
    @StateObject private var choices = TextChoicesModel()
    var body: some View {
        candidateEditor
            .task(id: revision) {
                await choices.load(view: view, column: column, query: query, after: after)
            }
            .onDisappear { choices.cancel() }
    }

    private var candidateEditor: some View {
        // Read the parent draft now; deferred Binding getters alone do not make
        // SwiftUI invalidate the checkbox list when selection changes.
        let selected = draft
        return VStack(alignment: .leading, spacing: 10) {
            TextField(
                "Find value",
                text: Binding(
                    get: { query },
                    set: {
                        query = $0
                        requestPage(after: nil)
                    })
            ).textFieldStyle(.roundedBorder)
            List {
                if let page = choices.page {
                    ForEach(page.values, id: \.literalIdentity) { item in
                        let identity = item.literalIdentity
                        HStack {
                            Toggle(
                                identity.label,
                                isOn: Binding(
                                    get: { selected.contains(identity) },
                                    set: { draft.set(identity, selected: $0) }
                                )
                            ).toggleStyle(.checkbox).lineLimit(1)
                            Spacer(minLength: 6)
                            Text(formatCount(item.count)).monospacedDigit().foregroundStyle(.secondary)
                        }.help(identity.label)
                    }
                }
            }.listStyle(.bordered).frame(height: 230)
            if choices.loading { ProgressView("Loading values…").controlSize(.small) }
            if let error = choices.error {
                HStack {
                    Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
                    Button("Retry") { revision += 1 }
                }
            }
            HStack {
                Button("First Page") { requestPage(after: nil) }.disabled(after == nil || choices.loading)
                Button("Next Page") {
                    requestPage(after: choices.page?.next)
                }.disabled(choices.page?.next == nil || choices.loading)
                Spacer()
                Text("\(choices.page?.values.count ?? 0) values on this page")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Select Page") { selectPage(true) }.disabled(choices.page == nil)
                Button("Clear Page") { selectPage(false) }.disabled(choices.page == nil)
            }.controlSize(.small)
        }
    }

    private func requestPage(after cursor: TextChoiceCursor?) {
        after = cursor
        choices.pause()
        revision += 1
    }

    private func selectPage(_ selected: Bool) {
        for item in choices.page?.values ?? [] { draft.set(item.literalIdentity, selected: selected) }
    }

}

extension TextChoice {
    fileprivate var literalIdentity: LiteralTextValue { LiteralTextValue(value: value) }
}
