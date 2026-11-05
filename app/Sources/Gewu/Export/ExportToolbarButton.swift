import SwiftUI

struct ExportToolbarButton: View {
    @ObservedObject var model: DocumentModel
    @ObservedObject var export: ExportModel

    var body: some View {
        Button {
            model.chooseExport()
        } label: {
            Label("Export", systemImage: "square.and.arrow.up")
        }
        .disabled(model.isBusy || !export.isIdle || model.appliedSpec.columns.isEmpty)
        .help("Export visible columns and filtered rows")
    }
}
