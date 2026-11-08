import SwiftUI

struct SortingView: View {
    @ObservedObject var model: DocumentModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Sort").font(.headline)
                Spacer()
                if !model.sortedColumns.isEmpty {
                    Text("First rule has priority").font(.caption).foregroundStyle(.secondary)
                }
            }
            if model.sortedColumns.isEmpty {
                Text("Choose a column to order the results.")
                    .foregroundStyle(.secondary).font(.callout)
            } else {
                List {
                    ForEach(Array(model.sortedColumns.enumerated()), id: \.element.column) { index, col in
                        HStack(spacing: 8) {
                            Image(systemName: "line.3.horizontal").foregroundStyle(.tertiary)
                                .help("Drag to change priority")
                            Picker(
                                "Column",
                                selection: Binding(
                                    get: { col.column },
                                    set: { model.replaceSortColumn(col.column, with: $0) }
                                )
                            ) {
                                ForEach(
                                    model.spec.columns.filter { $0.sort == nil || $0.column == col.column },
                                    id: \.column
                                ) {
                                    Text(model.info.columns[Int($0.column)].name).tag($0.column)
                                }
                            }.labelsHidden().frame(maxWidth: .infinity)
                            Picker(
                                "Direction",
                                selection: Binding(
                                    get: { col.sort?.descending ?? false },
                                    set: { model.setSortDirection(column: col.column, descending: $0) }
                                )
                            ) {
                                Text("Ascending").tag(false)
                                Text("Descending").tag(true)
                            }.labelsHidden().frame(width: 115)
                            Button {
                                model.removeSort(column: col.column)
                            } label: {
                                Image(systemName: "xmark").foregroundStyle(.secondary)
                            }.buttonStyle(.borderless).accessibilityLabel("Remove sort")
                        }
                        .contextMenu {
                            Button("Move Up") { move(index, by: -1) }.disabled(index == 0)
                            Button("Move Down") { move(index, by: 1) }
                                .disabled(index == model.sortedColumns.count - 1)
                        }
                        .accessibilityAction(named: "Move Up") { if index > 0 { move(index, by: -1) } }
                        .accessibilityAction(named: "Move Down") {
                            if index + 1 < model.sortedColumns.count { move(index, by: 1) }
                        }
                    }.onMove { from, to in
                        var columns = model.sortedColumns.map(\.column)
                        columns.move(fromOffsets: from, toOffset: to)
                        model.reorderSort(columns)
                    }
                }
                .listStyle(.plain).scrollContentBackground(.hidden)
                .frame(height: min(260, CGFloat(model.sortedColumns.count) * 38 + 8))
            }
            HStack {
                Menu {
                    ForEach(model.spec.columns.filter { $0.sort == nil }, id: \.column) { col in
                        Button(model.info.columns[Int(col.column)].name) {
                            model.setSortDirection(column: col.column, descending: false)
                        }
                    }
                } label: {
                    Label("Add Sort", systemImage: "plus")
                }
                .menuStyle(.borderlessButton).fixedSize()
                .disabled(model.spec.columns.allSatisfy { $0.sort != nil })
                Spacer()
                if !model.sortedColumns.isEmpty {
                    Button("Clear All Sorting") { model.clearSort() }.buttonStyle(.borderless)
                }
            }
        }
        .controlSize(.small).padding(20).frame(width: 430)
    }

    private func move(_ index: Int, by delta: Int) {
        var columns = model.sortedColumns.map(\.column)
        columns.swapAt(index, index + delta)
        model.reorderSort(columns)
    }
}
