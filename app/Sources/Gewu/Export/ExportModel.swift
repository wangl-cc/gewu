import AppKit
import GewuKit
import Observation
import SwiftUI
import UniformTypeIdentifiers

/// Owns the save panel and the export job captured from the displayed rows.
@MainActor
@Observable
final class ExportModel {
    enum Outcome: Equatable {
        case exported(String)
        case cancelled
    }

    private(set) var progress: ExportProgress?
    private(set) var outcome: Outcome?
    private(set) var choosingDestination = false

    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var panel: NSSavePanel?
    /// The panel's delegate, held for as long as the panel is open.
    @ObservationIgnored private var selection: ExportSelection?
    @ObservationIgnored private let alerts: Alerts
    @ObservationIgnored private let defaults: UserDefaults

    init(alerts: Alerts, defaults: UserDefaults = .standard) {
        self.alerts = alerts
        self.defaults = defaults
    }

    var isIdle: Bool { !choosingDestination && progress == nil }

    /// Ask for a destination and export `columns` of `rows` there.
    func choose(rows: DisplayRows, columns: [ColumnID], source: URL, window: NSWindow) {
        guard isIdle else { return }
        // Every format can export nested columns: flat formats write their display text.
        let formats = ExportFileFormat.allCases
        let remembered = ExportFileFormat(rawValue: defaults[Preferences.lastExportFormat])
        let initial =
            if defaults[Preferences.remembersExportFormat], let remembered, formats.contains(remembered) {
                remembered
            } else {
                ExportFileFormat.parquet
            }
        let compression = ParquetCompression(rawValue: defaults[Preferences.parquetCompression]) ?? .zstd
        let selection = ExportSelection(compression: compression)
        let panel = NSSavePanel()
        panel.title = "Export"
        panel.prompt = "Export"
        panel.message = "Export \(formatCount(rows.count)) rows · visible columns in display order"
        panel.nameFieldStringValue = source.deletingPathExtension().lastPathComponent + "-export.\(initial.rawValue)"
        panel.allowedContentTypes = formats.map(\.contentType)
        panel.currentContentType = initial.contentType
        panel.showsContentTypes = true
        panel.delegate = selection
        selection.panel(panel, didSelect: panel.currentContentType)
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        panel.showsTagField = false
        self.panel = panel
        self.selection = selection
        choosingDestination = true
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }
            let destination = response == .OK ? panel.url : nil
            let format = formats.first { $0.contentType == panel.currentContentType }
            self.choosingDestination = false
            self.panel = nil
            self.selection = nil
            guard let destination, let format else { return }
            self.start(
                rows: rows, columns: columns, format: format, compression: selection.compression, to: destination)
        }
    }

    private func start(
        rows: DisplayRows, columns: [ColumnID], format: ExportFileFormat, compression: ParquetCompression,
        to destination: URL
    ) {
        let job: ExportJob
        do {
            job = try rows.export(columns, format: format.fileType)
        } catch {
            alerts.report(error, title: "Could Not Export")
            return
        }
        outcome = nil
        progress = ExportProgress(rowsWritten: 0, totalRows: UInt64(rows.count))
        task = Task {
            let updates = Task {
                for await update in job.progress() { progress = update }
            }
            defer { updates.cancel() }
            do {
                try await job.write(to: destination, compression: compression)
                defaults[Preferences.lastExportFormat] = format.rawValue
                if format == .parquet { defaults[Preferences.parquetCompression] = compression.rawValue }
                outcome = .exported(destination.lastPathComponent)
            } catch is CancellationError {
                outcome = .cancelled
            } catch {
                alerts.report(error, title: "Could Not Export")
            }
            progress = nil
            task = nil
        }
    }

    /// Stop the running export; the destination is left untouched unless already written.
    func cancel() { task?.cancel() }

    func stop() {
        panel?.cancel(nil)
        cancel()
    }
}

/// The save panel's delegate: names formats and offers Parquet compression.
@MainActor
@Observable
private final class ExportSelection: NSObject, NSOpenSavePanelDelegate {
    var compression: ParquetCompression

    init(compression: ParquetCompression) {
        self.compression = compression
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
    @Bindable var selection: ExportSelection

    var body: some View {
        HStack(spacing: 8) {
            Text("Compression:")
            Picker("Compression", selection: $selection.compression) {
                ForEach(ParquetCompression.allCases, id: \.self) { compression in
                    Text(compression.title).tag(compression)
                }
            }
            .labelsHidden().frame(width: 160)
        }
        .fixedSize().padding(.vertical, 12).padding(.horizontal, 16)
    }
}
