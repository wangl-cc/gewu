import AppKit
import SwiftUI

/// Window-owned presentation state. Rust owns the data; SwiftUI observes only
/// metadata and view revisions, never an array containing the whole table.
@MainActor
final class DocumentModel: ObservableObject {
    let url: URL
    @Published private(set) var doc: GewuDocument?
    @Published private(set) var info: FileInfo?
    @Published private(set) var kinds: [ColumnKind] = []
    @Published private(set) var current = ViewInfo(version: 0, rowCount: 0)
    @Published private(set) var appliedState = ViewState(sort: [], filters: [], search: nil)
    @Published private(set) var isBusy = false
    @Published private(set) var errorMessage: String?
    @Published var isErrorPresented = false
    @Published var searchText = ""
    @Published var selectedColumn: Int?
    @Published var selectedSourceRow: UInt64?
    @Published var selectedDisplayRow: Int?
    @Published private(set) var sort: [SortKey] = []
    @Published private(set) var columnFilters: [UInt32: [Filter]] = [:]
    @Published var searchFocusRequest = 0
    @Published private(set) var exportProgress: ExportProgress?
    @Published private(set) var exportMessage: String?
    @Published private var choosingExport = false

    weak var table: DataTableController?
    private var viewTask: Task<Void, Never>?
    private var exportTask: Task<Void, Never>?
    private var exportJob: GewuExport?
    private var exportPanel: NSSavePanel?
    private var request: UInt64 = 0
    private let gate = RequestGate()
    // Serial execution keeps Rust's request order consistent with UI order.
    private let worker = DispatchQueue(label: "gewu.document.views", qos: .userInitiated)

    init(url: URL) { self.url = url }

    var canExport: Bool { doc != nil && !isBusy && !choosingExport && exportProgress == nil }

    func chooseExport() {
        guard canExport, let doc, let table, let window = table.view.window else { return }
        let version = current.version
        let columns = table.displayedColumns
        let nestedColumns: [String]
        do {
            nestedColumns = try doc.nestedExportColumns(columns: columns)
        } catch {
            reportError(error)
            return
        }
        let formats: [ExportFileFormat] = nestedColumns.isEmpty ? ExportFileFormat.allCases : [.parquet]
        let defaultFormat: ExportFileFormat = nestedColumns.isEmpty ? .csv : .parquet
        let panel = NSSavePanel()
        panel.title = "Export"
        panel.prompt = "Export"
        panel.message = "Export all \(formatCount(current.rowCount)) rows in the current results, in the displayed row and column order."
        panel.nameFieldStringValue = url.deletingPathExtension().lastPathComponent + "-export.\(defaultFormat.rawValue)"
        panel.allowedContentTypes = formats.map(\.contentType)
        panel.showsContentTypes = true
        panel.currentContentType = defaultFormat.contentType
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        panel.showsTagField = false
        exportPanel = panel
        choosingExport = true
        panel.beginSheetModal(for: window) { [weak self, weak panel] response in
            guard let self else { return }
            let destination = response == .OK ? panel?.url : nil
            let format = ExportFileFormat(contentType: panel?.currentContentType)
            self.choosingExport = false
            self.exportPanel = nil
            guard let destination else { return }
            guard let format else {
                self.reportError("Choose an available export format.")
                return
            }
            do {
                let job = try doc.prepareExport(version: version, columns: columns,
                                                format: format.coreFormat)
                self.startExport(job, to: destination)
            } catch {
                self.reportError(error)
            }
        }
    }

    private func startExport(_ job: GewuExport, to destination: URL) {
        exportJob = job
        exportProgress = job.progress()
        exportMessage = nil
        exportTask = Task { [weak self] in
            let polling = Task { [weak self] in
                while !Task.isCancelled {
                    self?.exportProgress = job.progress()
                    do { try await Task.sleep(for: .milliseconds(150)) }
                    catch { return }
                }
            }
            defer { polling.cancel() }
            do {
                try await Task.detached(priority: .userInitiated) { try job.write(path: destination.path) }.value
                self?.exportMessage = "Exported \(destination.lastPathComponent)"
            } catch GewuError.Cancelled {
                self?.exportMessage = "Export cancelled"
            } catch {
                self?.exportMessage = nil
                self?.reportError(error, prefix: "Could not export: ")
            }
            self?.exportProgress = nil
            self?.exportJob = nil
            self?.exportTask = nil
        }
    }

    func cancelExport() { exportJob?.cancel() }

    var state: ViewState {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return ViewState(sort: sort,
                         filters: columnFilters.sorted { $0.key < $1.key }.flatMap(\.value),
                         search: query.isEmpty ? nil : query)
    }

    var subtitle: String {
        guard let info else { return "" }
        let source = info.sheet.map { "\(info.format) · \($0)" } ?? info.format
        return "\(source) · \(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
            + ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)
    }

    var status: String {
        guard let info else { return isBusy ? "Opening \(url.lastPathComponent)…" : "Could not open file" }
        var text = current.rowCount == info.rowCount
            ? "\(formatCount(current.rowCount)) rows"
            : "\(formatCount(current.rowCount)) of \(formatCount(info.rowCount)) rows"
        text += " · \(info.columns.count) columns"
        if !columnFilters.isEmpty { text += " · \(columnFilters.count) filtered" }
        if let row = selectedSourceRow { text += " · row \(formatCount(row)) in file" }
        if appliedState.search != nil, current.rowCount > 0 {
            if let row = selectedDisplayRow {
                text += " · match \(formatCount(UInt64(row + 1))) of \(formatCount(current.rowCount))"
            } else {
                text += " · ⏎ next match · ⇧⏎ previous"
            }
        }
        return text
    }

    func open() async {
        guard doc == nil, !isBusy else { return }
        errorMessage = nil
        isErrorPresented = false
        isBusy = true
        do {
            let path = url.path
            let opened = try await Task.detached(priority: .userInitiated) {
                try GewuDocument.open(path: path)
            }.value
            try Task.checkCancellation()
            info = opened.info()
            kinds = info?.columns.map { ColumnKind($0.kind) } ?? []
            current = opened.currentView()
            selectedColumn = info?.columns.isEmpty == false ? 0 : nil
            doc = opened
            FileCoordinator.shared.recordRecent(url)
            isBusy = false
            // A query may have been entered while the file was decoding.
            if state.search != nil { refresh() }
            return
        } catch is CancellationError {
            // The window disappeared before decoding finished.
        } catch {
            reportError(error)
        }
        isBusy = false
    }

    func setSort(_ keys: [SortKey]) {
        guard keys != sort else { return }
        sort = keys
        refresh()
    }

    func setFilters(_ filters: [Filter], column: UInt32) {
        columnFilters[column] = filters.isEmpty ? nil : filters
        refresh()
    }

    func clearFilters() {
        guard !columnFilters.isEmpty else { return }
        columnFilters.removeAll()
        refresh()
    }

    func refresh(debounce: Bool = false) {
        guard let doc else { return }
        viewTask?.cancel()
        doc.cancelPending()
        request &+= 1
        let mine = request
        gate.set(mine)
        let state = state
        let worker = worker, gate = gate
        isBusy = true
        viewTask = Task { [weak self] in
            do {
                if debounce { try await Task.sleep(for: .milliseconds(150)) }
                try Task.checkCancellation()
                let view: ViewInfo = try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        guard gate.isCurrent(mine) else {
                            continuation.resume(throwing: CancellationError())
                            return
                        }
                        do { continuation.resume(returning: try doc.applyView(state: state)) }
                        catch { continuation.resume(throwing: error) }
                    }
                }
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.appliedState = state
                self.current = view
                self.selectedDisplayRow = nil
                self.selectedSourceRow = nil
                self.isBusy = false
            } catch {
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.isBusy = false
                if case GewuError.Cancelled = error { return }
                self.reportError(error)
            }
        }
    }

    func stop() {
        request &+= 1
        gate.set(request)
        viewTask?.cancel()
        doc?.cancelPending()
        exportPanel?.cancel(nil)
        cancelExport()
    }

    func findNext(backwards: Bool = false) { table?.moveToMatch(by: backwards ? -1 : 1) }
    func focusSearch() { searchFocusRequest &+= 1 }

    private func reportError(_ message: String) {
        errorMessage = message
        isErrorPresented = true
    }

    private func reportError(_ error: Error, prefix: String = "") {
        let message: String
        switch error {
        case let GewuError.Io(reason), let GewuError.Unsupported(reason),
             let GewuError.InvalidArgument(reason), let GewuError.Internal(reason):
            message = reason
        case GewuError.Cancelled:
            message = "The operation was cancelled."
        case GewuError.StaleVersion:
            message = "The results changed. Please try again."
        default:
            message = error.localizedDescription
        }
        reportError(prefix + message)
    }
}

/// The worker reads this before starting a queued synchronous FFI call.
private final class RequestGate: @unchecked Sendable {
    private let lock = NSLock()
    private var value: UInt64 = 0
    func set(_ value: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        self.value = value
    }
    func isCurrent(_ value: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return self.value == value
    }
}
