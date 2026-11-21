import GewuKit
import SwiftUI

extension FilterMode {
    var title: String {
        switch self {
        case .exact: "Select Values"
        case .range: "Range"
        case .text: "Text"
        }
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
    @State private var draft: FilterDraft
    /// The rows `choices` and `distribution` describe.
    @State private var describedRows: RowSet?
    @State private var choices: FilterChoices?
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
        _draft = State(initialValue: FilterDraft(current))
    }

    private var modes: [FilterMode] { FilterMode.available(capabilities: info.capabilities, choices: choices) }

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
                        ForEach(modes, id: \.self) { Text($0.title).tag(Optional($0)) }
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
                let result = try await rows.filterChoices(for: info.id)
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
            self.error = ErrorPresentation(error).text
        }
        loading = false
    }

    private func apply() {
        guard let mode else { return }
        onApply(draft.filter(mode: mode, hasContent: !modes.isEmpty))
    }
}
