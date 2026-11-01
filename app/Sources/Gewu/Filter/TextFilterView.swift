import SwiftUI

/// Swift String equality folds canonical Unicode equivalents. Filter values are
/// literal UTF-8 identities, including distinct null and empty-string values.
struct LiteralTextValue: Hashable {
    let value: String?

    static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs.value, rhs.value) {
        case (nil, nil): true
        case (let left?, let right?): left.utf8.elementsEqual(right.utf8)
        default: false
        }
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(value != nil)
        if let value { for byte in value.utf8 { hasher.combine(byte) } }
    }

    var label: String {
        guard let value else { return "(null)" }
        return value.isEmpty ? "(empty string)" : value
    }
}

struct TextChoiceDraft {
    var exclude = true
    var values: Set<LiteralTextValue> = []

    init(_ filters: [Filter]) {
        for filter in filters {
            switch filter {
            case .textValues(let values, let exclude):
                self.values = Set(values.map { LiteralTextValue(value: $0) })
                self.exclude = exclude
            case .isNull(let negate):
                values = [LiteralTextValue(value: nil)]
                exclude = negate
            case .equals(let value):
                values = [LiteralTextValue(value: value)]
                exclude = false
            case .equalsAny(let values):
                self.values = Set(values.map { LiteralTextValue(value: $0) })
                exclude = false
            default: break
            }
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

    func filters() -> [Filter] {
        if exclude && values.isEmpty { return [] }
        let sorted = values.map(\.value).sorted { lhs, rhs in
            switch (lhs, rhs) {
            case (nil, .some): true
            case (let left?, let right?): left.utf8.lexicographicallyPrecedes(right.utf8)
            default: false
            }
        }
        return [.textValues(values: sorted, exclude: exclude)]
    }
}

struct TextContainsDraft {
    var text = ""
    var caseSensitive = false
    var nullMode: NullMode

    init(_ filters: [Filter]) {
        nullMode = NullMode(filters)
        for case .contains(let text, let caseSensitive, _) in filters {
            self.text = text
            self.caseSensitive = caseSensitive
        }
    }

    func filters() -> [Filter] {
        if nullMode != .only && !text.isEmpty {
            return [
                .contains(
                    text: text, caseSensitive: caseSensitive,
                    includeNulls: nullMode == .include)
            ]
        }
        return nullMode.filter.map { [$0] } ?? []
    }
}

struct TextFilterView: View {
    let document: GewuDocument
    let column: UInt32
    let nullable: Bool
    let state: QuerySpec
    let modes: [TextFilterMode]
    let onApply: ([Filter]) -> Void
    @State private var mode: TextFilterMode
    @State private var choiceDraft: TextChoiceDraft
    @State private var containsDraft: TextContainsDraft
    @State private var query = ""
    @State private var after: TextChoiceCursor?
    @State private var revision = 0
    @StateObject private var choices = TextChoicesModel()

    init(
        document: GewuDocument, column: UInt32, nullable: Bool, state: QuerySpec,
        modes: [TextFilterMode], recommended: TextFilterMode, current: [Filter],
        onApply: @escaping ([Filter]) -> Void
    ) {
        self.document = document
        self.column = column
        self.nullable = nullable
        self.state = state
        self.modes = modes
        self.onApply = onApply
        var initial = recommended
        for filter in current {
            switch filter {
            case .contains: initial = .contains
            case .textValues, .isNull, .equals, .equalsAny: initial = .choices
            default: break
            }
        }
        _mode = State(initialValue: initial)
        _choiceDraft = State(initialValue: TextChoiceDraft(current))
        _containsDraft = State(initialValue: TextContainsDraft(current))
    }

    private struct Request: Equatable {
        let mode: TextFilterMode
        let revision: Int
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker("Filter mode", selection: $mode) {
                ForEach(modes, id: \.self) { mode in
                    Text(mode == .choices ? "Select Values" : "Contains Text").tag(mode)
                }
            }.pickerStyle(.segmented)
            switch mode {
            case .choices: candidateEditor
            case .contains: containsEditor
            }
        }
        .task(id: Request(mode: mode, revision: revision)) {
            guard mode == .choices else {
                choices.pause()
                return
            }
            await choices.load(document: document, column: column, state: state, query: query, after: after)
        }
        .onDisappear { choices.cancel() }
    }

    private var candidateEditor: some View {
        // Read the parent draft now; deferred Binding getters alone do not make
        // SwiftUI invalidate the checkbox list when selection changes.
        let selected = choiceDraft
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
                                    set: { choiceDraft.set(identity, selected: $0) }
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
                Button("All Values") { choiceDraft.all(true) }
                Button("No Values") { choiceDraft.all(false) }
                Spacer()
                Button("Select Page") { selectPage(true) }.disabled(choices.page == nil)
                Button("Clear Page") { selectPage(false) }.disabled(choices.page == nil)
            }.controlSize(.small)
            HStack {
                Text(selectionSummary).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Apply") { onApply(choiceDraft.filters()) }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private var selectionSummary: String {
        if choiceDraft.values.isEmpty { return choiceDraft.exclude ? "All values selected" : "No values selected" }
        let count = formatCount(UInt64(choiceDraft.values.count))
        return choiceDraft.exclude ? "All values except \(count)" : "\(count) values selected"
    }

    private func requestPage(after cursor: TextChoiceCursor?) {
        after = cursor
        choices.pause()
        revision += 1
    }

    private func selectPage(_ selected: Bool) {
        for item in choices.page?.values ?? [] { choiceDraft.set(item.literalIdentity, selected: selected) }
    }

    private var containsEditor: some View {
        ContainsEditor(column: column, nullable: nullable, draft: $containsDraft, onApply: onApply)
    }
}

extension TextChoice {
    fileprivate var literalIdentity: LiteralTextValue { LiteralTextValue(value: value) }
}
