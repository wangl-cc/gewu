import SwiftUI

struct ExportToolbarButton: View {
    @ObservedObject var model: DocumentModel
    @ObservedObject var export: ExportModel

    var body: some View {
        Button {
            chooseExport(for: model)
        } label: {
            Label("Export", systemImage: "square.and.arrow.up")
        }
        .disabled(!model.canExport)
        .help("Export visible columns and filtered rows")
    }
}

/// Resolve the presenting window in the UI layer; ExportModel owns the save panel and job.
@MainActor
func chooseExport(for model: DocumentModel) {
    guard model.canExport, let window = model.table?.view.window else { return }
    model.export.choose(
        view: model.view, columns: model.appliedSpec.columns.map(\.column), source: model.url, window: window)
}
