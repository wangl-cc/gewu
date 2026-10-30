import AppKit
import Combine
import SwiftUI

enum InspectorPage: String, CaseIterable {
    case column = "Column Overview"
    case chart = "Chart"
}

/// One loaded table's presentation state. Rust owns the data; SwiftUI observes only
/// metadata and view revisions, never an array containing the whole table.
@MainActor
final class DocumentModel: ObservableObject {
    let url: URL
    let doc: GewuDocument
    let info: FileInfo
    let kinds: [ColumnKind]
    @Published private(set) var current = ViewInfo(version: 0, rowCount: 0)
    @Published private(set) var appliedState = ViewState(sort: [], filters: [], search: nil)
    @Published private(set) var isBusy = false
    @Published private(set) var errorMessage: String?
    @Published var isErrorPresented = false
    @Published var searchText = ""
    @Published var selectedColumn: Int?
    @Published var inspectedColumn: Int?
    @Published var inspectorPage = InspectorPage.column
    @Published var chartX: Int?
    @Published var chartY: Int?
    @Published var selectedSourceRow: UInt64?
    @Published var selectedDisplayRow: Int?
    @Published private(set) var sort: [SortKey] = []
    @Published private(set) var columnFilters: [UInt32: [Filter]] = [:]
    @Published private(set) var appliedFilterColumns: Set<UInt32> = []
    @Published var searchFocusRequest = 0
    private(set) lazy var export = ExportModel { [weak self] error, prefix in
        self?.reportError(error, prefix: prefix)
    }
    private(set) lazy var copying = CopyModel { [weak self] error, prefix in
        self?.reportError(error, prefix: prefix)
    }

    weak var table: DataTableController?
    private var viewTask: Task<Void, Never>?
    private var request: UInt64 = 0
    private let gate = RequestGate()
    // Serial execution keeps Rust's request order consistent with UI order.
    private let worker = DispatchQueue(label: "gewu.document.views", qos: .userInitiated)

    init(url: URL, document: GewuDocument) {
        self.url = url
        doc = document
        let metadata = document.info()
        info = metadata
        kinds = metadata.columns.map { ColumnKind($0.kind) }
        current = document.currentView()
        selectedColumn = metadata.columns.isEmpty ? nil : 0
        inspectedColumn = selectedColumn
    }

    var canExport: Bool { !isBusy && export.isIdle }

    func chooseExport() {
        guard canExport, let table, let window = table.view.window else { return }
        export.choose(document: doc, view: current, columns: table.displayedColumns, source: url, window: window)
    }

    var state: ViewState {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return ViewState(
            sort: sort,
            filters: columnFilters.sorted { $0.key < $1.key }.flatMap(\.value),
            search: query.isEmpty ? nil : query)
    }

    var subtitle: String {
        let source = info.sheet.map { "\(info.format) · \($0)" } ?? info.format
        return "\(source) · \(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
            + ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)
    }

    func status(rowNumberBase: Int) -> String {
        var text =
            current.rowCount == info.rowCount
            ? "\(formatCount(current.rowCount)) rows"
            : "\(formatCount(current.rowCount)) of \(formatCount(info.rowCount)) rows"
        text += " · \(info.columns.count) columns"
        if !appliedFilterColumns.isEmpty { text += " · \(appliedFilterColumns.count) filtered" }
        if let row = selectedSourceRow { text += " · row \(formatRowNumber(row, base: rowNumberBase)) in file" }
        if appliedState.search != nil, current.rowCount > 0 {
            if let row = selectedDisplayRow {
                text += " · match \(formatCount(UInt64(row + 1))) of \(formatCount(current.rowCount))"
            } else {
                text += " · ⏎ next match · ⇧⏎ previous"
            }
        }
        return text
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

    var canClearFilters: Bool { !columnFilters.isEmpty || !appliedFilterColumns.isEmpty }

    func clearFilters() {
        guard canClearFilters else { return }
        columnFilters.removeAll()
        refresh()
    }

    func refresh(debounce: Bool = false) {
        let doc = doc
        viewTask?.cancel()
        doc.cancelPending()
        request &+= 1
        let mine = request
        gate.set(mine)
        let state = state
        let filterColumns = Set(columnFilters.keys)
        let worker = worker
        let gate = gate
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
                        do { continuation.resume(returning: try doc.applyView(state: state)) } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.copying.cancel()
                self.appliedFilterColumns = filterColumns
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
        doc.cancelPending()
        export.stop()
        copying.cancel()
        table?.stopLoading()
    }

    func findNext(backwards: Bool = false) { table?.moveToMatch(by: backwards ? -1 : 1) }
    func focusSearch() { searchFocusRequest &+= 1 }

    private func reportError(_ message: String) {
        errorMessage = message
        isErrorPresented = true
    }

    func reportError(_ error: Error, prefix: String = "") {
        reportError(prefix + error.userMessage)
    }
}

/// The worker reads this before starting a queued synchronous FFI call.
final class RequestGate: @unchecked Sendable {
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
