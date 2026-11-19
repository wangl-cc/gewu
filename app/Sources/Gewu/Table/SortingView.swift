import SwiftUI

struct SortingView: View {
    let model: DocumentModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Sort").font(.headline)
                Spacer()
                if !model.draftSorts.isEmpty {
                    Text("First rule has priority").font(.caption).foregroundStyle(.secondary)
                }
            }
            if model.draftSorts.isEmpty {
                Text("Choose a column to order the results.")
                    .foregroundStyle(.secondary).font(.callout)
            } else {
                List {
                    ForEach(Array(model.draftSorts.enumerated()), id: \.element.column) { index, sort in
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
                                    Text(model.columnInfo[column]?.name ?? "").tag(column)
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
                                .disabled(index == model.draftSorts.count - 1)
                        }
                        .accessibilityAction(named: "Move Up") { if index > 0 { move(index, by: -1) } }
                        .accessibilityAction(named: "Move Down") {
                            if index + 1 < model.draftSorts.count { move(index, by: 1) }
                        }
                    }.onMove { from, to in
                        var columns = model.draftSorts.map(\.column)
                        columns.move(fromOffsets: from, toOffset: to)
                        model.reorderSort(columns)
                    }
                }
                .listStyle(.plain).scrollContentBackground(.hidden)
                .frame(height: min(260, CGFloat(model.draftSorts.count) * 38 + 8))
            }
            HStack {
                Menu {
                    ForEach(unsortedVisibleColumns, id: \.self) { column in
                        Button(model.columnInfo[column]?.name ?? "") {
                            model.setSortDirection(column: column, descending: false)
                        }
                    }
                } label: {
                    Label("Add Sort", systemImage: "plus")
                }
                .menuStyle(.borderlessButton).fixedSize()
                .disabled(unsortedVisibleColumns.isEmpty)
                Spacer()
                if !model.draftSorts.isEmpty {
                    Button("Clear All Sorting") { model.clearSort() }.buttonStyle(.borderless)
                }
            }
        }
        .controlSize(.small).padding(20).frame(width: 430)
    }

    private var unsortedVisibleColumns: [ColumnId] {
        model.draftVisibleColumns.filter { id in !model.draftSorts.contains(where: { $0.column == id }) }
    }

    private func availableColumns(including current: ColumnId) -> [ColumnId] {
        model.draftVisibleColumns.filter { id in
            id == current || !model.draftSorts.contains(where: { $0.column == id })
        }
    }

    private func move(_ index: Int, by delta: Int) {
        var columns = model.draftSorts.map(\.column)
        columns.swapAt(index, index + delta)
        model.reorderSort(columns)
    }
}
