import SwiftUI

enum FilterMode: String, CaseIterable {
    case exact = "Select Values"
    case range = "Range"
    case contains = "Contains"

    /// A nil mode means the initial bounded candidate request is still needed.
    static func initial(
        capabilities: ColumnCapabilities, current: Filter?, choices: FilterChoices? = nil
    ) -> FilterMode? {
        if let current {
            switch current.condition {
            case .range: if capabilities.range { return .range }
            case .contains: if capabilities.contains { return .contains }
            case .exact: if capabilities.exact { return .exact }
            }
        }
        if capabilities.exact {
            guard let choices else { return nil }
            if choices.values != nil { return .exact }
        }
        return capabilities.range ? .range : (capabilities.contains ? .contains : .exact)
    }
}

struct ColumnFilterView: View {
    let view: GewuView
    let column: UInt32
    let info: ColumnInfo
    let onApply: (Filter?) -> Void
    @State private var mode: FilterMode?
    @State private var exact: ExactFilterDraft
    @State private var contains: TextContainsDraft
    @State private var range: RangeDraft
    @State private var nullMode: NullMode
    @State private var choices: FilterChoices?
    @State private var distribution: RangeHistogram?
    @State private var error: String?
    @State private var histogramBins: UInt32 = 60
    @State private var histogramScale: AxisScale = .linear
    @State private var literal = ""
    @State private var loading = false
    @State private var retry = 0

    init(
        view: GewuView, column: UInt32, info: ColumnInfo, current: Filter?,
        onApply: @escaping (Filter?) -> Void
    ) {
        self.view = view
        self.column = column
        self.info = info
        self.onApply = onApply
        _mode = State(initialValue: FilterMode.initial(capabilities: info.capabilities, current: current))
        _exact = State(initialValue: ExactFilterDraft(current))
        _contains = State(initialValue: TextContainsDraft(current))
        _range = State(initialValue: RangeDraft(current))
        _nullMode = State(initialValue: NullMode(current))
    }

    private var modes: [FilterMode] {
        FilterMode.allCases.filter {
            switch $0 {
            case .range: info.capabilities.range
            case .contains: info.capabilities.contains
            case .exact: info.capabilities.exact
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
                    case .contains: ContainsEditor(draft: $contains)
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
        } else if info.capabilities.contains {
            TextFilterView(view: view, column: column, draft: $exact)
        } else {
            HStack {
                TextField("Exact value", text: $literal).textFieldStyle(.roundedBorder)
                Button("Add") {
                    exact.values.insert(LiteralTextValue(value: literal))
                    literal = ""
                }
            }
            List {
                ForEach(exact.values.sorted { $0.value < $1.value }, id: \.self) { value in
                    HStack {
                        Text(value.label)
                        Spacer()
                        Button("Remove") { exact.values.remove(value) }
                    }
                }
            }.frame(height: 160)
            Toggle("Exclude these values", isOn: $exact.exclude)
        }
        HStack {
            Button("All Values") { exact.all(true) }
            Button("No Values") { exact.all(false) }
            Spacer()
            Text(exact.exclude ? "All except \(exact.values.count)" : "\(exact.values.count) selected").font(.caption)
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
        let view = view
        let column = column
        let mode = mode
        let bins = histogramBins
        let scale = histogramScale
        let cancellation = Cancellation()
        do {
            if mode == .range {
                let result = try await withTaskCancellationHandler {
                    try await Task.detached {
                        try view.filterDistribution(
                            column: column, bins: bins, scale: scale, cancellation: cancellation)
                    }.value
                } onCancel: {
                    cancellation.cancel()
                }
                guard !Task.isCancelled else { return }
                distribution = result
                range.restoreLabels(from: result)
            } else if mode == nil || (mode == .exact && choices == nil) {
                let result = try await withTaskCancellationHandler {
                    try await Task.detached {
                        try view.filterChoices(column: column, cancellation: cancellation)
                    }.value
                } onCancel: {
                    cancellation.cancel()
                }
                guard !Task.isCancelled else { return }
                choices = result
                if mode == nil {
                    self.mode = FilterMode.initial(capabilities: info.capabilities, current: nil, choices: result)
                }
            }
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.userMessage
        }
        if !Task.isCancelled { loading = false }
    }

    private func reset() {
        exact = ExactFilterDraft(nil)
        contains = TextContainsDraft(nil)
        range = RangeDraft(nil)
        nullMode = .include
        literal = ""
        error = nil
    }

    private func apply() {
        guard let mode else { return }
        let activeCondition: FilterCondition? =
            switch mode {
            case .range: range.condition
            case .contains: contains.condition
            case .exact: exact.condition
            }
        let filter = nullMode.applying(to: modes.isEmpty ? nil : activeCondition)
        onApply(filter)
    }
}
