import SwiftUI

struct SortingView: View {
    @ObservedObject var model: DocumentModel
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sort Columns").font(.headline)
            Text("Top to bottom priority. Shift-click a header to add a key; click again for descending, then no sort.")
                .font(.caption).foregroundStyle(.secondary)
            List {
                ForEach(Array(model.sortedColumns.enumerated()), id: \.element.column) { index, col in
                    HStack {
                        Text("\(index + 1). \(model.info.columns[Int(col.column)].name)")
                        Spacer()
                        Picker(
                            "Direction",
                            selection: Binding(
                                get: { col.sort?.descending ?? false },
                                set: { model.setSortDirection(column: col.column, descending: $0) })
                        ) {
                            Text("Ascending").tag(false)
                            Text("Descending").tag(true)
                        }.labelsHidden().frame(width: 115)
                        Button {
                            move(index, by: -1)
                        } label: {
                            Image(systemName: "arrow.up")
                        }.disabled(index == 0)
                        Button {
                            move(index, by: 1)
                        } label: {
                            Image(systemName: "arrow.down")
                        }.disabled(index == model.sortedColumns.count - 1)
                        Button("Remove") { model.removeSort(column: col.column) }
                    }
                }.onMove { from, to in
                    var columns = model.sortedColumns.map(\.column)
                    columns.move(fromOffsets: from, toOffset: to)
                    model.reorderSort(columns)
                }
            }.frame(height: 210)
            HStack {
                Menu("Add Column") {
                    ForEach(model.spec.columns.filter { $0.sort == nil }, id: \.column) { col in
                        Button(model.info.columns[Int(col.column)].name) {
                            model.setSortDirection(column: col.column, descending: false)
                        }
                    }
                }.disabled(model.spec.columns.allSatisfy { $0.sort != nil })
                Button("Clear All") { model.clearSort() }.disabled(model.sortedColumns.isEmpty)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(width: 560)
    }
    private func move(_ index: Int, by delta: Int) {
        var columns = model.sortedColumns.map(\.column)
        columns.swapAt(index, index + delta)
        model.reorderSort(columns)
    }
}
