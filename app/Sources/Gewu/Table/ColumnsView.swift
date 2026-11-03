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
                                if visible { model.showColumn(UInt32(index)) } else { model.hideColumn(UInt32(index)) }
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
