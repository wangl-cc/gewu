import Foundation
import GewuKit
import Observation

enum InspectorPage: String, CaseIterable {
    case column = "Column"
    case chart = "Chart"
}

/// How a window reads its file. Tests replace it to control timing.
struct TableLoader: Sendable {
    var worksheetNames: @Sendable (URL) async throws -> [String]
    var open: @Sendable (URL, OpenOptions) async throws -> TableSession

    static let live = TableLoader(
        worksheetNames: { try await TableSession.worksheetNames($0) },
        open: { try await TableSession.open($0, options: $1) })
}

/// One window: its file, worksheet and loading state, and presentation that
/// outlives a worksheet change. Each opened worksheet gets a fresh ``TableModel``.
@MainActor
@Observable
final class WindowModel {
    let url: URL
    let alerts = Alerts()
    /// Chart and histogram settings, kept across columns and worksheets.
    let inspectorSettings = InspectorSettings()

    private(set) var table: TableModel?
    private(set) var sheets: [WorksheetName] = []
    /// The worksheet being opened while the current table stays visible.
    private(set) var pendingSheet: WorksheetName?
    private(set) var isOpening = false
    /// Why the last open failed; shown while no table is open.
    private(set) var openError: String?
    private(set) var hasHeader: Bool
    var showsImportOptions = false
    var showsInspector: Bool {
        didSet { defaults[Preferences.showsInspector] = showsInspector }
    }
    var inspectorPage = InspectorPage.column
    /// Whether the Find field has keyboard focus. Setting it moves focus there.
    var isSearchFocused = false

    @ObservationIgnored private let loader: TableLoader
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let didOpen: @MainActor (URL) -> Void
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var listedSheets = false
    @ObservationIgnored private var attemptedSheet: WorksheetName?

    init(
        url: URL, hasHeader: Bool = true, loader: TableLoader = .live, defaults: UserDefaults = .standard,
        didOpen: @escaping @MainActor (URL) -> Void = { FileCoordinator.shared.recordRecent($0) }
    ) {
        self.url = url
        self.hasHeader = hasHeader
        self.loader = loader
        self.defaults = defaults
        self.didOpen = didOpen
        showsInspector = defaults[Preferences.showsInspector]
    }

    var selectedSheet: WorksheetName? {
        table?.info.worksheet.map(WorksheetName.init)
    }

    var supportsHeaderOption: Bool { ["csv", "tsv", "xlsx"].contains(url.pathExtension.lowercased()) }

    func open() {
        guard table == nil, !isOpening else { return }
        load(sheet: attemptedSheet)
    }

    func reopen(hasHeader: Bool) {
        guard supportsHeaderOption else { return }
        load(sheet: selectedSheet ?? attemptedSheet, hasHeader: hasHeader)
    }

    func selectSheet(_ sheet: WorksheetName) {
        guard sheets.contains(sheet) else { return }
        if sheet == selectedSheet {
            cancelOpening()
            return
        }
        guard !isOpening || pendingSheet != sheet else { return }
        load(sheet: sheet)
    }

    /// Analyze `column` and show it in the Inspector.
    func inspect(_ column: ColumnID) {
        table?.inspect(column)
        inspectorPage = .column
        showsInspector = true
    }

    func stop() {
        cancelOpening()
        table?.stop()
    }

    private func cancelOpening() {
        task?.cancel()
        isOpening = false
        pendingSheet = nil
    }

    /// Open `sheet`, keeping the current table until the replacement succeeds.
    /// Opens run one after another: a replaced open that has not started never
    /// reads the file, and a new one waits for a read already in progress.
    private func load(sheet: WorksheetName?, hasHeader: Bool? = nil) {
        let previous = task
        cancelOpening()
        attemptedSheet = sheet
        pendingSheet = sheet
        openError = nil
        isOpening = true
        let header = hasHeader ?? self.hasHeader
        let listsSheets = url.pathExtension.lowercased() == "xlsx" && !listedSheets
        let loader = loader
        let url = url
        task = Task {
            await previous?.value
            do {
                try Task.checkCancellation()
                var selected = sheet
                if listsSheets {
                    let names = try await loader.worksheetNames(url).map(WorksheetName.init)
                    try Task.checkCancellation()
                    // Keep the catalog even if the selected sheet cannot open; others stay selectable.
                    sheets = names
                    listedSheets = true
                    selected = selected ?? names.first
                    attemptedSheet = selected
                }
                let session = try await loader.open(url, OpenOptions(sheet: selected?.rawValue, hasHeader: header))
                try Task.checkCancellation()
                show(TableModel(url: url, session: session, alerts: alerts), hasHeader: header)
            } catch {
                // A replaced open reports nothing, even if it failed.
                guard !Task.isCancelled, !(error is CancellationError) else { return }
                isOpening = false
                pendingSheet = nil
                openError = ErrorPresentation(error).text
                alerts.report(error, title: "Could Not Open Table")
            }
        }
    }

    private func show(_ next: TableModel, hasHeader header: Bool) {
        table?.stop()
        table = next
        hasHeader = header
        if case .xlsx(_, let names) = next.info.metadata {
            sheets = names.map(WorksheetName.init)
        } else {
            sheets = []
        }
        pendingSheet = nil
        isOpening = false
        didOpen(url)
    }
}
