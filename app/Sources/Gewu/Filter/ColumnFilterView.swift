import SwiftUI

private enum FilterMode: String, CaseIterable {
    case range = "Range"
    case contains = "Contains"
    case exact = "Exact values"
}

struct ColumnFilterView: View {
    let doc: GewuSession
    let column: UInt32
    let info: ColumnInfo
    let querySpec: QuerySpec
    let onApply: (Filter?) -> Void
    @State private var mode: FilterMode
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

    init(
        doc: GewuSession, column: UInt32, info: ColumnInfo, querySpec: QuerySpec, current: Filter?,
        onApply: @escaping (Filter?) -> Void
    ) {
        self.doc = doc
        self.column = column
        self.info = info
        self.querySpec = querySpec
        self.onApply = onApply
        var initial: FilterMode = info.capabilities.range ? .range : (info.capabilities.contains ? .contains : .exact)
        if let current {
            switch current.condition {
            case .range: initial = .range
            case .contains: initial = .contains
            case .exact: if info.capabilities.exact { initial = .exact }
            }
        }
        _mode = State(initialValue: initial)
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
            if !modes.isEmpty {
                Picker("Filter mode", selection: $mode) {
                    ForEach(modes, id: \.self) { Text($0.rawValue).tag($0) }
                }.pickerStyle(.segmented)
                Group {
                    switch mode {
                    case .range:
                        RangeFilterView(
                            doc: doc, column: column, distribution: distribution, bins: $histogramBins,
                            scale: $histogramScale, supportsLog: info.capabilities.logarithmic, draft: $range)
                    case .contains: ContainsEditor(draft: $contains)
                    case .exact: exactEditor
                    }
                }.disabled(nullMode == .only)
            }
            if loading { ProgressView().controlSize(.small) }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                NullModePicker(selection: $nullMode, nullCount: choices?.nullCount)
                Spacer()
                Button("Clear") { onApply(nil) }
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
        }.padding(14).frame(width: 440)
            .task(id: Request(mode: mode, bins: histogramBins, scale: histogramScale)) { await load() }
    }

    @ViewBuilder private var exactEditor: some View {
        if let values = choices?.values {
            ChoiceFilterView(values: values, draft: $exact)
        } else if info.valueCategory == .text {
            TextFilterView(document: doc, column: column, querySpec: querySpec, draft: $exact)
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
        let mode: FilterMode
        let bins: UInt32
        let scale: AxisScale
    }
    @MainActor private func load() async {
        loading = true
        error = nil
        let doc = doc
        let column = column
        let spec = querySpec
        let mode = mode
        let bins = histogramBins
        let scale = histogramScale
        let cancellation = Cancellation()
        do {
            if mode == .range {
                let result = try await withTaskCancellationHandler {
                    try await Task.detached {
                        try doc.filterDistribution(
                            column: column, spec: spec, bins: bins, scale: scale, cancellation: cancellation)
                    }.value
                } onCancel: {
                    cancellation.cancel()
                }
                guard !Task.isCancelled else { return }
                distribution = result
                range.restoreLabels(from: result)
            } else if mode == .exact, choices == nil {
                let result = try await withTaskCancellationHandler {
                    try await Task.detached {
                        try doc.filterChoices(column: column, spec: spec, cancellation: cancellation)
                    }.value
                } onCancel: {
                    cancellation.cancel()
                }
                guard !Task.isCancelled else { return }
                choices = result
            }
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.userMessage
        }
        if !Task.isCancelled { loading = false }
    }

    private func apply() {
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
