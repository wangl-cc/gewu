import SwiftUI

struct ColumnFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let info: ColumnInfo
    let state: ViewState
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var spec: FilterSpec?
    @State private var error: String?
    @State private var histogramBins: UInt32 = 60
    @State private var loading = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text(info.name).font(.headline)
                Text(shortTypeName(info.typeName)).font(.caption).foregroundStyle(.secondary)
            }
            if let spec {
                editor(spec)
                if loading { ProgressView().controlSize(.small) }
                if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            } else if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            } else {
                ProgressView("Loading filter…").frame(maxWidth: .infinity, minHeight: 120)
            }
        }
        .padding(14)
        .frame(width: 420)
        .task(id: histogramBins) { await load() }
    }

    @ViewBuilder private func editor(_ spec: FilterSpec) -> some View {
        switch spec.editor {
        case .text(let modes, let recommended):
            TextFilterView(
                document: doc, column: column, nullable: spec.nullable, state: state,
                modes: modes, recommended: recommended, current: current, onApply: onApply)
        case .range(_, let distribution):
            RangeFilterView(
                doc: doc, column: column, nullable: spec.nullable,
                distribution: distribution, bins: $histogramBins, current: current, onApply: onApply)
        case .choices(let values):
            ChoiceFilterView(column: column, values: values, current: current, onApply: onApply)
        case .contains:
            ContainsFilterView(column: column, nullable: spec.nullable, current: current, onApply: onApply)
        case .nullOnly:
            NullFilterView(column: column, current: current, onApply: onApply)
        }
    }

    @MainActor private func load() async {
        loading = true
        error = nil
        let bins = histogramBins
        do {
            let doc = doc
            let column = column
            let state = state
            let spec = try await Task.detached { try doc.filterSpecWithBins(column: column, state: state, bins: bins) }
                .value
            guard !Task.isCancelled else { return }
            self.spec = spec
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
        if !Task.isCancelled { loading = false }
    }
}
