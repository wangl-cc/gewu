import GewuKit
import SwiftUI

struct ColumnsView: View {
    let model: TableModel
    @State private var search = ""

    private var matching: [ColumnInfo] {
        model.source.columns.filter {
            search.isEmpty || $0.name.localizedCaseInsensitiveContains(search)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Visible Columns").font(.headline)
            TextField("Find column", text: $search).textFieldStyle(.roundedBorder)
            List {
                ForEach(matching, id: \.id) { column in
                    Toggle(
                        isOn: Binding(
                            get: { model.visibleColumns.contains(column.id) },
                            set: { visible in
                                if visible {
                                    model.setColumnsVisible([column.id], visible: true)
                                } else {
                                    hideColumn(column.id, in: model)
                                }
                            })
                    ) {
                        HStack {
                            Text(column.name)
                            Spacer()
                            Text(column.displayType).foregroundStyle(.secondary).font(.caption)
                        }
                    }.toggleStyle(.checkbox)
                }
            }.listStyle(.plain).scrollContentBackground(.hidden)
            HStack {
                Text("\(model.visibleColumns.count) of \(model.source.columns.count) visible").font(.caption)
                Spacer()
                Menu {
                    Button("Show Matching Columns") {
                        model.setColumnsVisible(matching.map(\.id), visible: true)
                    }
                    Button("Hide Matching Columns") { hideColumns(matching.map(\.id), in: model) }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton).fixedSize().disabled(matching.isEmpty)
                .accessibilityLabel("Matching column actions")
                Button("Show All") { model.showAllColumns() }
                    .buttonStyle(.borderless)
                    .disabled(model.visibleColumns.count == model.source.columns.count)
            }
        }.padding(16).frame(width: 340, height: min(380, 120 + CGFloat(model.source.columns.count) * 28))
    }
}

/// Shared by the column picker and the AppKit header menu. The model only applies the confirmed change.
@MainActor
func hideColumn(_ column: ColumnID, in model: TableModel) {
    hideColumns([column], in: model)
}

@MainActor
func hideColumns(_ columns: [ColumnID], in model: TableModel) {
    let targetSet = Set(columns)
    let targets = model.visibleColumns.filter { targetSet.contains($0) }
    guard !targets.isEmpty else { return }
    let hasConditions = targets.contains { col in
        model.draft.filter(for: col) != nil
            || model.appliedQuery.filter(for: col) != nil
            || model.draft.sort(for: col) != nil
            || model.appliedQuery.sort(for: col) != nil
    }
    if hasConditions, !UserDefaults.standard[Preferences.skipsHideColumnWarning] {
        let alert = NSAlert()
        alert.messageText =
            targets.count == 1
            ? "Hide \(model.column(targets[0])?.name ?? "")?" : "Hide \(targets.count) columns?"
        alert.informativeText =
            "Hiding columns clears their sorting and filters. This may change the visible rows and their order."
        alert.addButton(withTitle: targets.count == 1 ? "Hide Column" : "Hide Columns")
        alert.addButton(withTitle: "Cancel")
        alert.showsSuppressionButton = true
        alert.suppressionButton?.title = "Do not ask again"
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        if alert.suppressionButton?.state == .on {
            UserDefaults.standard[Preferences.skipsHideColumnWarning] = true
        }
    }
    model.setColumnsVisible(columns, visible: false)
}
