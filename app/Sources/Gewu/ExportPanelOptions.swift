import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The save panel owns the format picker; this delegate supplies only the
/// options that apply to that format and the captured export columns.
@MainActor
final class ExportPanelOptions: NSObject, ObservableObject, NSOpenSavePanelDelegate {
    @Published var convertNestedToJson = true
    private let nestedColumns: [String]

    init(nestedColumns: [String]) {
        self.nestedColumns = nestedColumns
    }

    func attach(to panel: NSSavePanel) {
        panel.delegate = self
        updateAccessory(in: panel, contentType: panel.currentContentType)
    }

    func panel(_ sender: Any, didSelect type: UTType?) {
        guard let panel = sender as? NSSavePanel else { return }
        updateAccessory(in: panel, contentType: type)
    }

    func panel(_ sender: Any, validate url: URL) throws {
        guard let panel = sender as? NSSavePanel,
              let format = ExportFileFormat(contentType: panel.currentContentType),
              format.supportsJSONConversion,
              !nestedColumns.isEmpty,
              !convertNestedToJson else { return }
        throw NSError(
            domain: "Gewu.Export", code: 1,
            userInfo: [
                NSLocalizedDescriptionKey: "These columns need JSON conversion to export as \(format.rawValue.uppercased()).",
                NSLocalizedRecoverySuggestionErrorKey: "Select “Convert lists and structs to JSON”, or choose Parquet to preserve the nested data."
            ]
        )
    }

    private func updateAccessory(in panel: NSSavePanel, contentType: UTType?) {
        guard !nestedColumns.isEmpty,
              ExportFileFormat(contentType: contentType)?.supportsJSONConversion == true else {
            panel.accessoryView = nil
            return
        }
        let accessory = NSHostingView(rootView: JSONExportOptionsView(options: self))
        accessory.setFrameSize(accessory.fittingSize)
        panel.accessoryView = accessory
    }
}

private struct JSONExportOptionsView: View {
    @ObservedObject var options: ExportPanelOptions

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Toggle("Convert lists and structs to JSON", isOn: $options.convertNestedToJson)
                .toggleStyle(.checkbox)
            Text(options.convertNestedToJson
                 ? "Lists and structs are saved as JSON text in a single cell."
                 : "Enable conversion, or choose Parquet to keep the nested data.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(width: 380, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.vertical, 8)
    }
}
