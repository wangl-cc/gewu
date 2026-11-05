import SwiftUI

struct ColumnsView: View {
    @ObservedObject var model: DocumentModel
    @State private var search = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Visible Columns").font(.headline)
            TextField("Find column", text: $search).textFieldStyle(.roundedBorder)
            List {
                ForEach(
                    model.info.columns.indices.filter {
                        search.isEmpty || model.info.columns[$0].name.localizedCaseInsensitiveContains(search)
                    }, id: \.self
                ) { index in
                    Toggle(
                        isOn: Binding(
                            get: { model.spec.columns.contains { $0.column == UInt32(index) } },
                            set: { visible in
                                if visible {
                                    model.showColumn(UInt32(index))
                                } else {
                                    hideColumn(UInt32(index), in: model)
                                }
                            })
                    ) {
                        HStack {
                            Text(model.info.columns[index].name)
                            Spacer()
                            Text(model.info.columns[index].displayType).foregroundStyle(.secondary).font(.caption)
                        }
                    }.toggleStyle(.checkbox)
                }
            }.listStyle(.bordered)
            HStack {
                Text("\(model.spec.columns.count) of \(model.info.columns.count) visible").font(.caption)
                Spacer()
                Button("Show All") { model.showAllColumns() }
                    .disabled(model.spec.columns.count == model.info.columns.count)
            }
        }.padding(14).frame(width: 330, height: 350)
    }
}

/// Shared by the column picker and the AppKit header menu. The model only applies the confirmed change.
@MainActor
func hideColumn(_ column: UInt32, in model: DocumentModel) {
    guard let col = model.spec.columns.first(where: { $0.column == column }) else { return }
    if col.sort != nil || col.filter != nil, !UserDefaults.standard.bool(forKey: "SkipHideColumnWarning") {
        let alert = NSAlert()
        alert.messageText = "Hide \(model.info.columns[Int(column)].name)?"
        alert.informativeText =
            "Hiding this column clears its sorting and filter. This may change the visible rows and their order."
        alert.addButton(withTitle: "Hide Column")
        alert.addButton(withTitle: "Cancel")
        alert.showsSuppressionButton = true
        alert.suppressionButton?.title = "Do not ask again"
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        if alert.suppressionButton?.state == .on {
            UserDefaults.standard.set(true, forKey: "SkipHideColumnWarning")
        }
    }
    model.removeColumn(column)
}
