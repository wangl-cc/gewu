import Combine
import Foundation

/// Window state survives worksheet changes; each loaded table owns its own
/// query, selection, analysis settings and caches in a fresh DocumentModel.
@MainActor
final class WorkbookModel: ObservableObject {
    let url: URL
    @Published private(set) var document: DocumentModel?
    @Published private(set) var sheets: [String] = []
    @Published private(set) var pendingSheet: String?
    @Published private(set) var isOpening = false
    @Published private(set) var errorMessage: String?
    @Published var isErrorPresented = false
    @Published var showsInspector = UserDefaults.standard.bool(forKey: "ShowColumnInspector")
    @Published var showsImportOptions = false
    @Published private(set) var hasHeader: Bool

    private var task: Task<Void, Never>?
    private var request: UInt64 = 0
    private let gate = RequestGate()
    private let worker = DispatchQueue(label: "gewu.workbook.open", qos: .userInitiated)
    private var listedSheets = false
    private var attemptedSheet: String?
    private let sheetNames: @Sendable (String) throws -> [String]
    private let openDocument: @Sendable (String, String?, Bool) throws -> GewuDocument
    private let didOpen: @MainActor (URL) -> Void

    init(
        url: URL,
        hasHeader: Bool = true,
        sheetNames: @escaping @Sendable (String) throws -> [String] = { try xlsxSheetNames(path: $0) },
        openDocument: @escaping @Sendable (String, String?, Bool) throws -> GewuDocument = { path, sheet, header in
            try GewuDocument.openWithOptions(path: path, sheet: sheet, hasHeader: header)
        },
        didOpen: @escaping @MainActor (URL) -> Void = { FileCoordinator.shared.recordRecent($0) }
    ) {
        self.url = url
        self.hasHeader = hasHeader
        self.sheetNames = sheetNames
        self.openDocument = openDocument
        self.didOpen = didOpen
    }

    var selectedSheet: String? { document?.info.sheet }

    var supportsHeaderOption: Bool { ["csv", "tsv", "xlsx"].contains(url.pathExtension.lowercased()) }

    func reopen(hasHeader: Bool) {
        guard supportsHeaderOption else { return }
        load(sheet: selectedSheet ?? attemptedSheet, hasHeader: hasHeader)
    }

    func open() {
        guard document == nil, !isOpening else { return }
        load(sheet: attemptedSheet)
    }

    func selectSheet(_ sheet: String) {
        guard sheets.contains(sheet) else { return }
        if sheet == selectedSheet {
            cancelOpening()
            return
        }
        guard !isOpening || pendingSheet != sheet else { return }
        load(sheet: sheet)
    }

    func stop() {
        cancelOpening()
        document?.stop()
    }

    private func cancelOpening() {
        task?.cancel()
        task = nil
        request &+= 1
        gate.set(request)
        isOpening = false
        pendingSheet = nil
    }

    private func load(sheet: String?, hasHeader: Bool? = nil) {
        cancelOpening()
        attemptedSheet = sheet
        pendingSheet = sheet
        errorMessage = nil
        isErrorPresented = false
        isOpening = true
        let mine = request
        let path = url.path
        let listSheets = url.pathExtension.lowercased() == "xlsx" && !listedSheets
        let gate = gate
        let worker = worker
        let sheetNames = sheetNames
        let openDocument = openDocument
        let header = hasHeader ?? self.hasHeader
        task = Task { [weak self] in
            do {
                let loaded: WorkbookRead = try await withCheckedThrowingContinuation { continuation in
                    worker.async {
                        guard gate.isCurrent(mine) else {
                            continuation.resume(throwing: CancellationError())
                            return
                        }
                        do {
                            let names = try listSheets ? sheetNames(path) : nil
                            let selected = sheet ?? names?.first
                            guard gate.isCurrent(mine) else { throw CancellationError() }
                            // Keep the successful catalog even when the selected sheet
                            // contains an Excel error. Other sheets remain selectable.
                            let document = Result { try openDocument(path, selected, header) }
                            continuation.resume(
                                returning: WorkbookRead(sheets: names, sheet: selected, document: document))
                        } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                guard !Task.isCancelled, let self, self.request == mine else { return }
                if let names = loaded.sheets {
                    self.sheets = names
                    self.listedSheets = true
                }
                self.attemptedSheet = loaded.sheet
                let opened = try loaded.document.get()
                let next = DocumentModel(url: self.url, document: opened)
                self.document?.stop()
                self.document = next
                self.hasHeader = header
                self.sheets = next.info.sheets
                self.pendingSheet = nil
                self.isOpening = false
                self.task = nil
                self.didOpen(self.url)
            } catch {
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.pendingSheet = nil
                self.isOpening = false
                self.task = nil
                if error is CancellationError { return }
                self.errorMessage = error.userMessage
                self.isErrorPresented = true
            }
        }
    }

}

private struct WorkbookRead: Sendable {
    let sheets: [String]?
    let sheet: String?
    let document: Result<GewuDocument, Error>
}
