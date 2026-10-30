import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

/// Owns the save panel and the export job captured from a document view.
@MainActor
final class ExportModel: ObservableObject {
    @Published private(set) var progress: ExportProgress?
    @Published private(set) var message: String?
    @Published private var choosingExport = false

    private var task: Task<Void, Never>?
    private var job: GewuExport?
    private var panel: NSSavePanel?
    private let reportError: (Error, String) -> Void

    init(reportError: @escaping (Error, String) -> Void) {
        self.reportError = reportError
    }

    var isIdle: Bool { !choosingExport && progress == nil }

    func choose(document doc: GewuDocument, view: ViewInfo, columns: [UInt32], source: URL, window: NSWindow) {
        guard isIdle else { return }
        let version = view.version
        let nestedColumns: [String]
        do {
            nestedColumns = try doc.nestedExportColumns(columns: columns)
        } catch {
            reportError(error, "")
            return
        }
        let formats: [ExportFileFormat] = nestedColumns.isEmpty ? ExportFileFormat.allCases : [.parquet]
        let remembered = ExportFileFormat(rawValue: UserDefaults.standard.string(forKey: "LastExportFormat") ?? "")
        let rememberFormat = UserDefaults.standard.object(forKey: "RememberExportFormat") as? Bool ?? true
        let defaultFormat: ExportFileFormat
        if rememberFormat, let remembered, formats.contains(remembered) {
            defaultFormat = remembered
        } else {
            defaultFormat = .parquet
        }
        let selection = ExportSelection()
        let panel = NSSavePanel()
        panel.title = "Export"
        panel.prompt = "Export"
        panel.message =
            "Export all \(formatCount(view.rowCount)) rows in the current results, in the displayed row and column order."
        panel.nameFieldStringValue =
            source.deletingPathExtension().lastPathComponent + "-export.\(defaultFormat.rawValue)"
        panel.allowedContentTypes = formats.map(\.contentType)
        panel.currentContentType = defaultFormat.contentType
        panel.showsContentTypes = true
        panel.delegate = selection
        selection.panel(panel, didSelect: panel.currentContentType)
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        panel.showsTagField = false
        self.panel = panel
        choosingExport = true
        panel.beginSheetModal(for: window) { [weak self, weak panel] response in
            guard let self else { return }
            let destination = response == .OK ? panel?.url : nil
            let format = formats.first { $0.contentType == panel?.currentContentType }
            self.choosingExport = false
            self.panel = nil
            guard let destination else { return }
            guard let format else {
                self.reportError(GewuError.InvalidArgument(message: "Select a supported export format."), "")
                return
            }
            do {
                let job = try doc.prepareExport(
                    version: version, columns: columns,
                    format: format.coreFormat)
                self.startExport(job, to: destination, format: format, compression: selection.compression)
            } catch {
                self.reportError(error, "")
            }
        }
    }

    private func startExport(
        _ job: GewuExport, to destination: URL, format: ExportFileFormat, compression: ExportCompression
    ) {
        self.job = job
        progress = job.progress()
        message = nil
        task = Task { [weak self] in
            let polling = Task { [weak self] in
                while !Task.isCancelled {
                    self?.progress = job.progress()
                    do { try await Task.sleep(for: .milliseconds(150)) } catch { return }
                }
            }
            defer { polling.cancel() }
            do {
                let options = ExportOptions(parquetCompression: compression.coreCompression)
                try await Task.detached(priority: .userInitiated) {
                    try job.writeWithOptions(path: destination.path, options: options)
                }.value
                UserDefaults.standard.set(format.rawValue, forKey: "LastExportFormat")
                if format == .parquet {
                    UserDefaults.standard.set(compression.rawValue, forKey: "ParquetCompression")
                }
                self?.message = "Exported \(destination.lastPathComponent)"
            } catch GewuError.Cancelled {
                self?.message = "Export cancelled"
            } catch {
                self?.message = nil
                self?.reportError(error, "Could not export: ")
            }
            self?.progress = nil
            self?.job = nil
            self?.task = nil
        }
    }

    func cancel() { job?.cancel() }

    func stop() {
        panel?.cancel(nil)
        cancel()
    }
}

@MainActor
private final class ExportSelection: NSObject, ObservableObject, NSOpenSavePanelDelegate {
    @Published var compression: ExportCompression

    override init() {
        compression =
            ExportCompression(rawValue: UserDefaults.standard.string(forKey: "ParquetCompression") ?? "") ?? .zstd
        super.init()
    }

    func panel(_ sender: Any, displayNameFor type: UTType) -> String? {
        ExportFileFormat.allCases.first { $0.contentType == type }?.title
    }

    func panel(_ sender: Any, didSelect type: UTType?) {
        guard let panel = sender as? NSSavePanel else { return }
        panel.accessoryView =
            type == ExportFileFormat.parquet.contentType
            ? NSHostingView(rootView: ExportAccessory(selection: self)) : nil
    }
}

private struct ExportAccessory: View {
    @ObservedObject var selection: ExportSelection

    var body: some View {
        Form {
            Picker("Compression", selection: $selection.compression) {
                ForEach(ExportCompression.allCases, id: \.self) { compression in
                    Text(compression.title).tag(compression)
                }
            }
        }
        .padding()
        .frame(width: 320)
    }
}
