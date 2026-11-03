import SwiftUI

enum FilterMode: String, CaseIterable {
    case exact = "Select Values"
    case range = "Range"
    case text = "Text"

    /// Value selection is offered only when the column has a complete list of values,
    /// so a nil mode means the bounded candidate request is still needed.
    static func initial(
        capabilities: ColumnCapabilities, current: Filter?, choices: FilterChoices? = nil
    ) -> FilterMode? {
        let selectable: Bool? = capabilities.exact ? choices.map { $0.values != nil } : false
        if let current {
            switch current.condition {
            case .range: if capabilities.range { return .range }
            case .text: if capabilities.textMatch { return .text }
            case .exact:
                guard let selectable else { return nil }
                if selectable { return .exact }
            }
        }
        guard let selectable else { return nil }
        if selectable { return .exact }
        return capabilities.range ? .range : (capabilities.textMatch ? .text : .exact)
    }
}

/// The rows a column's filter editor describes: the applied filters on the other
/// columns, without sorting. The column's own filter never hides its alternatives.
struct FilterFacet {
    let view: GewuView
    let spec: QuerySpec
    let column: UInt32

    /// Without its own filter, the column's facet is the applied view itself.
    var usesAppliedView: Bool {
        !spec.columns.contains { $0.column == column && $0.filter != nil }
    }

    /// May run a query over the whole dataset; call off the main actor.
    func resolve(cancellation: Cancellation) throws -> GewuView {
        if usesAppliedView { return view }
        let others = spec.columns.compactMap { entry -> ColSpec? in
            guard entry.column != column, let filter = entry.filter else { return nil }
            return ColSpec(column: entry.column, sort: nil, filter: filter)
        }
        return try view.query(spec: QuerySpec(columns: others), cancellation: cancellation)
    }
}

struct ColumnFilterView: View {
    let facet: FilterFacet
    let column: UInt32
    let info: ColumnInfo
    let current: Filter?
    let onApply: (Filter?) -> Void
    @State private var mode: FilterMode?
    @State private var exact: ExactFilterDraft
    @State private var textMatch: TextMatchDraft
    @State private var range: RangeDraft
    @State private var nullMode: NullMode
    @State private var choices: FilterChoices?
    @State private var distribution: RangeHistogram?
    @State private var error: String?
    @State private var histogramBins: UInt32 = 60
    @State private var histogramScale: AxisScale = .linear
    @State private var loading = false
    @State private var retry = 0
    /// Resolved once per editor; candidates and distributions all describe it.
    @State private var facetView: GewuView?

    init(
        facet: FilterFacet, info: ColumnInfo, current: Filter?,
        onApply: @escaping (Filter?) -> Void
    ) {
        self.facet = facet
        self.column = facet.column
        self.info = info
        self.current = current
        self.onApply = onApply
        _mode = State(initialValue: FilterMode.initial(capabilities: info.capabilities, current: current))
        _exact = State(initialValue: ExactFilterDraft(current))
        _textMatch = State(initialValue: TextMatchDraft(current))
        _range = State(initialValue: RangeDraft(current))
        _nullMode = State(initialValue: NullMode(current))
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
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(info.name).font(.headline)
                Text(info.displayType).font(.caption).foregroundStyle(.secondary)
            }
            if let mode, !modes.isEmpty {
                Picker("Filter mode", selection: $mode) {
                    ForEach(modes, id: \.self) { Text($0.rawValue).tag(Optional($0)) }
                }.pickerStyle(.segmented)
                Group {
                    switch mode {
                    case .range:
                        RangeFilterView(
                            distribution: distribution, bins: $histogramBins,
                            scale: $histogramScale, supportsLog: info.capabilities.logarithmic, draft: $range)
                    case .text: TextMatchEditor(draft: $textMatch)
                    case .exact: exactEditor
                    }
                }.disabled(nullMode == .only)
            }
            if mode == nil && error == nil {
                ProgressView("Loading filter…").frame(maxWidth: .infinity, minHeight: 120)
            } else if loading {
                ProgressView().controlSize(.small)
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if mode == nil && error != nil {
                Button("Retry") { retry += 1 }
            }
            HStack {
                NullModePicker(selection: $nullMode, nullCount: choices?.nullCount)
                Spacer()
                Button("Reset", action: reset)
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }.disabled(mode == nil)
        }.padding(14).frame(width: 440)
            .task(id: Request(mode: mode, bins: histogramBins, scale: histogramScale, retry: retry)) { await load() }
    }

    @ViewBuilder private var exactEditor: some View {
        if let values = choices?.values {
            ChoiceFilterView(values: values, draft: $exact)
            HStack {
                Button("All Values") { exact.all(true) }
                Button("No Values") { exact.all(false) }
                Spacer()
                Text(exact.exclude ? "All except \(exact.values.count)" : "\(exact.values.count) selected")
                    .font(.caption)
            }
        }
    }

    private struct Request: Equatable {
        let mode: FilterMode?
        let bins: UInt32
        let scale: AxisScale
        let retry: Int
    }
    @MainActor private func load() async {
        if let mode, !modes.contains(mode) { return }
        loading = true
        error = nil
        let facet = facet
        let column = column
        let current = current
        let mode = mode
        let bins = histogramBins
        let scale = histogramScale
        let cancellation = Cancellation()
        do {
            let view: GewuView
            if let facetView {
                view = facetView
            } else {
                view = try await runCancellable(cancellation: cancellation) {
                    try facet.resolve(cancellation: cancellation)
                }
                guard !Task.isCancelled else { return }
                facetView = view
            }
            // Choices decide whether value selection is offered and supply the null count.
            if info.capabilities.exact && choices == nil {
                let result = try await runCancellable(cancellation: cancellation) {
                    try view.filterChoices(column: column, cancellation: cancellation)
                }
                guard !Task.isCancelled else { return }
                choices = result
                if mode == nil {
                    // Choosing a mode restarts this task for that editor.
                    self.mode = FilterMode.initial(capabilities: info.capabilities, current: current, choices: result)
                }
            }
            if mode == .range {
                let result = try await runCancellable(cancellation: cancellation) {
                    try view.filterDistribution(
                        column: column, bins: bins, scale: scale, cancellation: cancellation)
                }
                guard !Task.isCancelled else { return }
                distribution = result
                range.restoreLabels(from: result)
            }
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.userMessage
        }
        if !Task.isCancelled { loading = false }
    }

    private func reset() {
        exact = ExactFilterDraft(nil)
        textMatch = TextMatchDraft(nil)
        range = RangeDraft(nil)
        nullMode = .include
        error = nil
    }

    private func apply() {
        guard let mode else { return }
        let activeCondition: FilterCondition? =
            switch mode {
            case .range: range.condition
            case .text: textMatch.condition
            case .exact: exact.condition
            }
        let filter = nullMode.applying(to: modes.isEmpty ? nil : activeCondition)
        onApply(filter)
    }
}
