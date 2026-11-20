import GewuKit
import SwiftUI

enum FilterMode: String, CaseIterable {
    case exact = "Select Values"
    case range = "Range"
    case text = "Text"

    /// Value selection is offered only when the column has a complete list of values,
    /// so a nil mode means the bounded candidate request is still needed.
    static func initial(
        capabilities: ColumnCapabilities, current: Filter?, choices: FilterOptions? = nil
    ) -> FilterMode? {
        let selectable: Bool? = capabilities.exact ? choices.map { $0.values != nil } : false
        if let current {
            switch current.condition {
            case .range: if capabilities.range { return .range }
            case .text: if capabilities.textMatch { return .text }
            case .exact:
                guard let selectable else { return nil }
                if selectable { return .exact }
            case nil: break
            }
        }
        guard let selectable else { return nil }
        if selectable { return .exact }
        return capabilities.range ? .range : (capabilities.textMatch ? .text : .exact)
    }
}

/// Editing state is private to the popover; only Apply publishes a filter.
struct ColumnFilterDraft {
    var exact: ExactFilterDraft
    var text: TextMatchDraft
    var range: RangeDraft
    var nullMode: NullMode

    init(_ filter: Filter?) {
        exact = ExactFilterDraft(filter)
        text = TextMatchDraft(filter)
        range = RangeDraft(filter)
        nullMode = NullMode(filter)
    }

    var includesMissing: Bool {
        get { nullMode != .exclude }
        set { nullMode = newValue ? .include : .exclude }
    }

    mutating func setValue(_ value: LiteralTextValue, selected: Bool) {
        if nullMode == .only {
            exact.all(false)
            nullMode = .include
        }
        exact.set(value, selected: selected)
    }

    mutating func selectAllValues(_ selected: Bool) {
        exact.all(selected)
        includesMissing = selected
    }

    mutating func reset() { self = Self(nil) }

    mutating func enterValues() {
        if nullMode == .only {
            exact.all(false)
            nullMode = .include
        }
    }

    func filter(mode: FilterMode, hasContent: Bool = true) -> Filter? {
        let condition: FilterCondition? =
            switch mode {
            case .exact: exact.condition
            case .text: text.condition
            case .range: range.condition
            }
        return nullMode.applying(to: hasContent ? condition : nil)
    }
}

/// Edits one column's filter against the table's current rows. Candidate
/// values and the range preview describe the rows passing every other filter.
struct ColumnFilterView: View {
    let model: TableModel
    let info: ColumnInfo
    let current: Filter?
    let onApply: (Filter?) -> Void
    @State private var mode: FilterMode?
    @State private var draft: ColumnFilterDraft
    /// The rows `choices` and `distribution` describe.
    @State private var describedRows: RowSet?
    @State private var choices: FilterOptions?
    @State private var distribution: RangeHistogram?
    @State private var error: String?
    // The fixed-width filter plot needs a useful preview, not a bin-count editor.
    private let histogramBins: UInt32 = 40
    @State private var histogramScale: AxisScale = .linear
    @State private var loading = false
    @State private var retry = 0

    init(
        model: TableModel, info: ColumnInfo, current: Filter?,
        onApply: @escaping (Filter?) -> Void
    ) {
        self.model = model
        self.info = info
        self.current = current
        self.onApply = onApply
        _mode = State(initialValue: FilterMode.initial(capabilities: info.capabilities, current: current))
        _draft = State(initialValue: ColumnFilterDraft(current))
    }

    private var modes: [FilterMode] {
        FilterMode.allCases.filter {
            switch $0 {
            case .range: info.capabilities.range
            case .text: info.capabilities.textMatch
            case .exact: info.capabilities.exact && choices?.values != nil
            }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(info.name).font(.headline)
                Spacer()
                if loading { ProgressView().controlSize(.small) }
                Text(info.displayType).font(.caption).foregroundStyle(.secondary)
            }
            if let mode, !modes.isEmpty {
                if modes.count > 1 {
                    Picker("Filter mode", selection: $mode) {
                        ForEach(modes, id: \.self) { Text($0.rawValue).tag(Optional($0)) }
                    }.pickerStyle(.segmented).labelsHidden().frame(maxWidth: .infinity)
                }
                Group {
                    switch mode {
                    case .range:
                        RangeFilterView(
                            distribution: distribution,
                            scale: $histogramScale, supportsLog: info.capabilities.logarithmic, draft: $draft.range
                        )
                        .disabled(draft.nullMode == .only)
                    case .text: TextMatchEditor(draft: $draft.text).disabled(draft.nullMode == .only)
                    case .exact: exactEditor
                    }
                }
            }
            if mode == nil && error == nil {
                loadingEditor
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if mode == nil && error != nil {
                Button("Retry") { retry += 1 }
            }
            Divider()
            HStack(spacing: 12) {
                if mode != .exact || modes.isEmpty {
                    NullModePicker(selection: $draft.nullMode, nullCount: choices?.nullCount)
                }
                Button("Reset") { draft.reset() }
                    .buttonStyle(.borderless).help("Reset all draft conditions; Apply to update the table")
                Spacer()
                Button("Apply", action: apply).keyboardShortcut(.defaultAction).disabled(mode == nil)
            }
        }.padding(16).frame(width: 380)
            .background(Color(nsColor: .windowBackgroundColor))
            .onChange(of: mode) { _, mode in
                if mode == .exact { draft.enterValues() }
            }
            .task(
                id: Request(
                    rows: model.displayed.rowSet, mode: mode, bins: histogramBins, scale: histogramScale, retry: retry)
            ) { await load(model.displayed.rowSet) }
    }

    @ViewBuilder private var exactEditor: some View {
        if let values = choices?.values {
            ChoiceFilterView(values: values, nullCount: choices?.nullCount ?? 0, draft: $draft)
        }
    }

    private var loadingEditor: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                ProgressView().controlSize(.small)
                Text("Loading values…").font(.caption).foregroundStyle(.secondary)
            }
            // Reserve a bounded editor body while complete choices decide the initial mode.
            ForEach(0..<4, id: \.self) { _ in
                RoundedRectangle(cornerRadius: 3).fill(.quaternary.opacity(0.5)).frame(height: 20)
            }
        }.frame(maxWidth: .infinity, minHeight: 180, alignment: .topLeading)
    }

    private struct Request: Equatable {
        let rows: RowSet
        let mode: FilterMode?
        let bins: UInt32
        let scale: AxisScale
        let retry: Int
    }
    private func load(_ rows: RowSet) async {
        if describedRows !== rows {
            // The table's rows changed while the editor was open; describe the new ones.
            describedRows = rows
            choices = nil
            distribution = nil
        }
        loading = true
        error = nil
        let mode = mode
        do {
            // Choices decide whether value selection is offered and supply the null count.
            if info.capabilities.exact && choices == nil {
                let result = try await rows.filterOptions(for: info.id)
                try Task.checkCancellation()
                choices = result
                if mode == nil {
                    // Choosing a mode restarts this task for that editor.
                    self.mode = FilterMode.initial(capabilities: info.capabilities, current: current, choices: result)
                }
            }
            if mode == .range {
                let result = try await rows.filterDistribution(info.id, bins: histogramBins, scale: histogramScale)
                try Task.checkCancellation()
                distribution = result
                draft.range.restoreLabels(from: result)
            }
        } catch {
            guard !Task.isCancelled, !(error is CancellationError) else { return }
            self.error = error.localizedDescription
        }
        loading = false
    }

    private func apply() {
        guard let mode else { return }
        onApply(draft.filter(mode: mode, hasContent: !modes.isEmpty))
    }
}
