import SwiftUI

struct ColumnsView: View {
    @ObservedObject var model: DocumentModel
    @State private var search = ""

    private var matching: [Int] {
        model.info.columns.indices.filter {
            search.isEmpty || model.info.columns[$0].name.localizedCaseInsensitiveContains(search)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Visible Columns").font(.headline)
            TextField("Find column", text: $search).textFieldStyle(.roundedBorder)
            List {
                ForEach(
                    matching, id: \.self
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
            }.listStyle(.plain).scrollContentBackground(.hidden)
            HStack {
                Text("\(model.spec.columns.count) of \(model.info.columns.count) visible").font(.caption)
                Spacer()
                Menu {
                    Button("Show Matching Columns") {
                        model.setColumnsVisible(matching.map(UInt32.init), visible: true)
                    }
                    Button("Hide Matching Columns") { hideColumns(matching.map(UInt32.init), in: model) }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton).fixedSize().disabled(matching.isEmpty)
                .accessibilityLabel("Matching column actions")
                Button("Show All") { model.showAllColumns() }
                    .buttonStyle(.borderless)
                    .disabled(model.spec.columns.count == model.info.columns.count)
            }
        }.padding(16).frame(width: 340, height: min(380, 120 + CGFloat(model.info.columns.count) * 28))
    }
}

/// Shared by the column picker and the AppKit header menu. The model only applies the confirmed change.
@MainActor
func hideColumn(_ column: UInt32, in model: DocumentModel) {
    hideColumns([column], in: model)
}

@MainActor
func hideColumns(_ columns: [UInt32], in model: DocumentModel) {
    let targets = model.spec.columns.filter { columns.contains($0.column) }
    guard !targets.isEmpty else { return }
    if targets.contains(where: { $0.sort != nil || $0.filter != nil }),
        !UserDefaults.standard.bool(forKey: "SkipHideColumnWarning")
    {
        let alert = NSAlert()
        alert.messageText =
            targets.count == 1
            ? "Hide \(model.info.columns[Int(targets[0].column)].name)?" : "Hide \(targets.count) columns?"
        alert.informativeText =
            "Hiding columns clears their sorting and filters. This may change the visible rows and their order."
        alert.addButton(withTitle: targets.count == 1 ? "Hide Column" : "Hide Columns")
        alert.addButton(withTitle: "Cancel")
        alert.showsSuppressionButton = true
        alert.suppressionButton?.title = "Do not ask again"
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        if alert.suppressionButton?.state == .on {
            UserDefaults.standard.set(true, forKey: "SkipHideColumnWarning")
        }
    }
    model.setColumnsVisible(columns, visible: false)
}
