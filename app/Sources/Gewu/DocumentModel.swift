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
    @Published var errorMessage: String?
    @Published var searchText = ""
    @Published var selectedColumn: Int?
    @Published var selectedSourceRow: UInt64?
    @Published var selectedDisplayRow: Int?
    @Published private(set) var sort: [SortKey] = []
    @Published private(set) var columnFilters: [UInt32: [Filter]] = [:]
    @Published var searchFocusRequest = 0

    weak var table: DataTableController?
    private var viewTask: Task<Void, Never>?
    private var request: UInt64 = 0
    private let gate = RequestGate()
    // Serial execution keeps Rust's request order consistent with UI order.
    private let worker = DispatchQueue(label: "gewu.document.views", qos: .userInitiated)

    init(url: URL) { self.url = url }

    var state: ViewState {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return ViewState(sort: sort,
                         filters: columnFilters.sorted { $0.key < $1.key }.flatMap(\.value),
                         search: query.isEmpty ? nil : query)
    }

    var subtitle: String {
        guard let info else { return "" }
        return "\(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
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
        if appliedState.search != nil, let row = selectedDisplayRow {
            text += " · match \(formatCount(UInt64(row + 1))) of \(formatCount(current.rowCount))"
        }
        return text
    }

    func open() async {
        guard doc == nil, !isBusy else { return }
        isBusy = true
        do {
            let path = url.path
            let opened = try await Task.detached(priority: .userInitiated) {
                try GewuDocument.open(path: path)
            }.value
            try Task.checkCancellation()
            info = opened.info()
            kinds = info?.columns.map { ColumnKind(typeName: $0.typeName) } ?? []
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
            errorMessage = error.localizedDescription
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
                self.errorMessage = error.localizedDescription
            }
        }
    }

    func stop() {
        request &+= 1
        gate.set(request)
        viewTask?.cancel()
        doc?.cancelPending()
    }

    func findNext(backwards: Bool = false) { table?.moveToMatch(by: backwards ? -1 : 1) }
    func focusSearch() { searchFocusRequest &+= 1 }
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
