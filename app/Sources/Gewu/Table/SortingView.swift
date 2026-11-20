import GewuKit
import SwiftUI

struct SortingView: View {
    let model: TableModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Sort").font(.headline)
                Spacer()
                if !model.draft.sorts.isEmpty {
                    Text("First rule has priority").font(.caption).foregroundStyle(.secondary)
                }
            }
            if model.draft.sorts.isEmpty {
                Text("Choose a column to order the results.")
                    .foregroundStyle(.secondary).font(.callout)
            } else {
                List {
                    ForEach(Array(model.draft.sorts.enumerated()), id: \.element.column) { index, sort in
                        HStack(spacing: 8) {
                            Image(systemName: "line.3.horizontal").foregroundStyle(.tertiary)
                                .help("Drag to change priority")
                            Picker(
                                "Column",
                                selection: Binding(
                                    get: { sort.column },
                                    set: { model.replaceSortColumn(sort.column, with: $0) }
                                )
                            ) {
                                ForEach(availableColumns(including: sort.column), id: \.self) { column in
                                    Text(model.column(column)?.name ?? "").tag(column)
                                }
                            }.labelsHidden().frame(maxWidth: .infinity)
                            Picker(
                                "Direction",
                                selection: Binding(
                                    get: { sort.descending },
                                    set: { model.setSortDirection(column: sort.column, descending: $0) }
                                )
                            ) {
                                Text("Ascending").tag(false)
                                Text("Descending").tag(true)
                            }.labelsHidden().frame(width: 115)
                            Button {
                                model.removeSort(column: sort.column)
                            } label: {
                                Image(systemName: "xmark").foregroundStyle(.secondary)
                            }.buttonStyle(.borderless).accessibilityLabel("Remove sort")
                        }
                        .contextMenu {
                            Button("Move Up") { move(index, by: -1) }.disabled(index == 0)
                            Button("Move Down") { move(index, by: 1) }
                                .disabled(index == model.draft.sorts.count - 1)
                        }
                        .accessibilityAction(named: "Move Up") { if index > 0 { move(index, by: -1) } }
                        .accessibilityAction(named: "Move Down") {
                            if index + 1 < model.draft.sorts.count { move(index, by: 1) }
                        }
                    }.onMove { from, to in
                        var columns = model.draft.sorts.map(\.column)
                        columns.move(fromOffsets: from, toOffset: to)
                        model.reorderSorts(columns)
                    }
                }
                .listStyle(.plain).scrollContentBackground(.hidden)
                .frame(height: min(260, CGFloat(model.draft.sorts.count) * 38 + 8))
            }
            HStack {
                Menu {
                    ForEach(unsortedVisibleColumns, id: \.self) { column in
                        Button(model.column(column)?.name ?? "") {
                            model.setSortDirection(column: column, descending: false)
                        }
                    }
                } label: {
                    Label("Add Sort", systemImage: "plus")
                }
                .menuStyle(.borderlessButton).fixedSize()
                .disabled(unsortedVisibleColumns.isEmpty)
                Spacer()
                if !model.draft.sorts.isEmpty {
                    Button("Clear All Sorting") { model.clearSorts() }.buttonStyle(.borderless)
                }
            }
        }
        .controlSize(.small).padding(20).frame(width: 430)
    }

    private var unsortedVisibleColumns: [ColumnID] {
        model.visibleColumns.filter { id in !model.draft.sorts.contains(where: { $0.column == id }) }
    }

    private func availableColumns(including current: ColumnID) -> [ColumnID] {
        model.visibleColumns.filter { id in
            id == current || !model.draft.sorts.contains(where: { $0.column == id })
        }
    }

    private func move(_ index: Int, by delta: Int) {
        var columns = model.draft.sorts.map(\.column)
        columns.swapAt(index, index + delta)
        model.reorderSorts(columns)
    }
}
