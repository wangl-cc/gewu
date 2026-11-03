import SwiftUI

/// Summaries belong to the accepted query, never to an in-flight draft.
struct AppliedFiltersView: View {
    @ObservedObject var model: DocumentModel

    var body: some View {
        if !model.appliedFilterColumns.isEmpty {
            HStack(spacing: 8) {
                Image(systemName: "line.3.horizontal.decrease").foregroundStyle(.secondary)
                ScrollView(.horizontal) {
                    HStack(spacing: 8) {
                        ForEach(model.appliedSpec.columns.filter { $0.filter != nil }, id: \.column) { col in
                            if let filter = col.filter {
                                let name = model.info.columns[Int(col.column)].name
                                HStack(spacing: 4) {
                                    Button {
                                        model.table?.showColumnFilter(col.column)
                                    } label: {
                                        Text("\(name): \(filter.summary)").lineLimit(1)
                                    }.help("Edit \(name): \(filter.summary)")
                                    Button {
                                        model.setFilter(nil, column: col.column)
                                    } label: {
                                        Image(systemName: "xmark.circle.fill")
                                    }.accessibilityLabel("Remove filter for \(name)")
                                }
                                .buttonStyle(.borderless)
                                .padding(.horizontal, 9).padding(.vertical, 5)
                                .background(.quaternary, in: Capsule())
                            }
                        }
                    }
                }.scrollIndicators(.hidden)
                Button("Clear All") { model.clearFilters() }.buttonStyle(.borderless)
            }
            .disabled(model.isBusy)
            .font(.caption).padding(.horizontal, 12).padding(.vertical, 6)
            Divider()
        }
    }
}

extension Filter {
    var summary: String {
        let content: String
        switch condition {
        case .range(let min, let max):
            switch (min, max) {
            case (.some(let lo), .some(let hi)): content = "\(lo) … \(hi)"
            case (.some(let lo), nil): content = "≥ \(lo)"
            case (nil, .some(let hi)): content = "≤ \(hi)"
            case (nil, nil): content = "All values"
            }
        case .contains(let text, let caseSensitive):
            content = "Contains “\(text)”" + (caseSensitive ? " (match case)" : "")
        case .exact(let values, let exclude):
            if values.isEmpty {
                if !exclude { return includeNulls ? "Only nulls" : "No values" }
                return includeNulls ? "All values" : "Not null"
            }
            let labels = values.prefix(2).map { "“\($0)”" }.joined(separator: ", ")
            let more = values.count > 2 ? " +\(values.count - 2)" : ""
            content = (exclude ? "Except " : "") + labels + more
        }
        return content + (includeNulls ? " · include nulls" : " · hide nulls")
    }
}
