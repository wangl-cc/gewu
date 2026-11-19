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

struct ColumnFilterView: View {
    let viewer: Viewer
    let info: ColumnInfo
    let current: Filter?
    let onApply: (Filter?) -> Void
    @State private var mode: FilterMode?
    @State private var draft: ColumnFilterDraft
    @State private var choices: FilterChoices?
    @State private var distribution: RangeHistogram?
    @State private var error: String?
    // The fixed-width filter plot needs a useful preview, not a bin-count editor.
    private let histogramBins: UInt32 = 40
    @State private var histogramScale: AxisScale = .linear
    @State private var loading = false
    @State private var retry = 0

    init(
        viewer: Viewer, info: ColumnInfo, current: Filter?,
        onApply: @escaping (Filter?) -> Void
    ) {
        self.viewer = viewer
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
            .task(id: Request(mode: mode, bins: histogramBins, scale: histogramScale, retry: retry)) { await load() }
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
        let mode: FilterMode?
        let bins: UInt32
        let scale: AxisScale
        let retry: Int
    }
    @MainActor private func load() async {
        loading = true
        error = nil
        let mode = mode
        do {
            // Choices decide whether value selection is offered and supply the null count.
            if info.capabilities.exact && choices == nil {
                let result = try await viewer.filterChoices(info.id)
                guard !Task.isCancelled else { return }
                choices = result
                if mode == nil {
                    // Choosing a mode restarts this task for that editor.
                    self.mode = FilterMode.initial(capabilities: info.capabilities, current: current, choices: result)
                }
            }
            if mode == .range {
                let result = try await viewer.filterDistribution(info.id, bins: histogramBins, scale: histogramScale)
                guard !Task.isCancelled else { return }
                distribution = result
                draft.range.restoreLabels(from: result)
            }
        } catch {
            guard !Task.isCancelled else { return }
            if error.isCancellation {
                // The applied View changed while this editor was open; describe the new one.
                choices = nil
                retry &+= 1
                return
            }
            self.error = error.userMessage
        }
        if !Task.isCancelled { loading = false }
    }

    private func apply() {
        guard let mode else { return }
        onApply(draft.filter(mode: mode, hasContent: !modes.isEmpty))
    }
}
