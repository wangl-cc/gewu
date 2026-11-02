import AppKit
import Combine
import SwiftUI

public typealias GewuDocument = GewuSession

enum InspectorPage: String, CaseIterable {
    case column = "Column Overview"
    case chart = "Chart"
}

/// One loaded table's presentation state. Rust owns the data; SwiftUI observes only
/// metadata and view revisions, never an array containing the whole table.
@MainActor
final class DocumentModel: ObservableObject {
    let url: URL
    let doc: GewuSession
    let info: FileInfo
    let kinds: [ValueCategory]
    let find: FindModel
    @Published private(set) var current = ViewInfo(version: 0, rowCount: 0)
    @Published private(set) var appliedSpec = QuerySpec(columns: [])
    @Published private(set) var isBusy = false
    @Published private(set) var errorMessage: String?
    @Published var isErrorPresented = false
    @Published var selectedColumn: Int?
    @Published var inspectedColumn: Int?
    @Published var inspectorPage = InspectorPage.column
    @Published var chartX: Int?
    @Published var chartY: Int?
    @Published var selectedSourceRow: UInt64?
    @Published var selectedDisplayRow: Int?
    @Published private(set) var spec = QuerySpec(columns: [])
    var appliedFilterColumns: Set<UInt32> {
        Set(appliedSpec.columns.filter { $0.filter != nil }.map(\.column))
    }
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

    init(url: URL, document: GewuSession) {
        self.url = url
        doc = document
        let metadata = document.info()
        info = metadata
        kinds = metadata.columns.map(\.valueCategory)
        find = FindModel()
        current = document.currentView()
        spec = QuerySpec(columns: metadata.columns.indices.map { ColSpec(column: UInt32($0), sort: nil, filter: nil) })
        appliedSpec = spec
        selectedColumn = metadata.columns.isEmpty ? nil : 0
        inspectedColumn = selectedColumn
        find.onNavigate = { [weak self] match in self?.table?.navigateToMatch(match) }
        find.onError = { [weak self] error, prefix in self?.reportError(error, prefix: prefix) }
    }

    var canExport: Bool { !isBusy && export.isIdle && !appliedSpec.columns.isEmpty }

    func chooseExport() {
        guard canExport, let table, let window = table.view.window else { return }
        export.choose(
            document: doc, view: current, columns: appliedSpec.columns.map(\.column), source: url, window: window)
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
        text += " · \(appliedSpec.columns.count) columns"
        if !appliedFilterColumns.isEmpty { text += " · \(appliedFilterColumns.count) filtered" }
        if let row = selectedSourceRow { text += " · row \(formatRowNumber(row, base: rowNumberBase)) in file" }
        if let findStatus = find.status { text += " · \(findStatus)" }
        return text
    }

    var sortedColumns: [ColSpec] {
        spec.columns.filter { $0.sort != nil }.sorted { $0.sort!.priority < $1.sort!.priority }
    }

    func cycleSort(column: UInt32, multiple: Bool) {
        guard let index = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        let old = spec.columns[index].sort
        if !multiple { for i in spec.columns.indices { spec.columns[i].sort = nil } }
        if old?.descending == true {
            spec.columns[index].sort = nil
        } else {
            let priority = multiple ? (old?.priority ?? UInt32(sortedColumns.count)) : 0
            spec.columns[index].sort = ColSort(priority: priority, descending: old != nil)
        }
        normalizeSort()
        refresh()
    }

    func setSortDirection(column: UInt32, descending: Bool) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        spec.columns[i].sort = ColSort(
            priority: spec.columns[i].sort?.priority ?? UInt32(sortedColumns.count), descending: descending)
        refresh()
    }

    func removeSort(column: UInt32) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        spec.columns[i].sort = nil
        normalizeSort()
        refresh()
    }

    func clearSort() {
        for i in spec.columns.indices { spec.columns[i].sort = nil }
        refresh()
    }

    func reorderSort(_ columns: [UInt32]) {
        guard Set(columns) == Set(sortedColumns.map(\.column)), columns.count == sortedColumns.count else { return }
        for (priority, column) in columns.enumerated() {
            if let i = spec.columns.firstIndex(where: { $0.column == column }) {
                spec.columns[i].sort?.priority = UInt32(priority)
            }
        }
        refresh()
    }

    private func normalizeSort() {
        for (priority, column) in sortedColumns.enumerated() {
            let i = spec.columns.firstIndex { $0.column == column.column }!
            spec.columns[i].sort?.priority = UInt32(priority)
        }
    }

    func setFilter(_ filter: Filter?, column: UInt32) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        spec.columns[i].filter = filter
        refresh()
    }

    var canClearFilters: Bool { spec.columns.contains { $0.filter != nil } || !appliedFilterColumns.isEmpty }

    func clearFilters() {
        guard canClearFilters else { return }
        for i in spec.columns.indices { spec.columns[i].filter = nil }
        refresh()
    }

    func reorderColumns(_ columns: [UInt32]) {
        guard columns != spec.columns.map(\.column), Set(columns) == Set(spec.columns.map(\.column)),
            columns.count == spec.columns.count
        else { return }
        spec.columns = columns.compactMap { column in spec.columns.first { $0.column == column } }
        refresh()
    }

    func hideColumn(_ column: UInt32) {
        guard let col = spec.columns.first(where: { $0.column == column }) else { return }
        if col.sort != nil || col.filter != nil, !UserDefaults.standard.bool(forKey: "SkipHideColumnWarning") {
            let alert = NSAlert()
            alert.messageText = "Hide \(info.columns[Int(column)].name)?"
            alert.informativeText =
                "Hiding this column clears its sorting and filter. This may change the visible rows and their order."
            alert.addButton(withTitle: "Hide Column")
            alert.addButton(withTitle: "Cancel")
            alert.showsSuppressionButton = true
            alert.suppressionButton?.title = "Do not ask again"
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            if alert.suppressionButton?.state == .on {
                UserDefaults.standard.set(true, forKey: "SkipHideColumnWarning")
            }
        }
        removeColumn(column)
    }

    /// Apply a hide operation after any required confirmation.
    func removeColumn(_ column: UInt32) {
        spec.columns.removeAll { $0.column == column }
        normalizeSort()
        if selectedColumn == Int(column) { selectedColumn = spec.columns.first.map { Int($0.column) } }
        refresh()
    }

    func showColumn(_ column: UInt32) {
        guard Int(column) < info.columns.count, !spec.columns.contains(where: { $0.column == column }) else { return }
        spec.columns.append(ColSpec(column: column, sort: nil, filter: nil))
        refresh()
    }

    func refresh(debounce: Bool = false) {
        let doc = doc
        find.invalidate()
        viewTask?.cancel()
        doc.cancelPending()
        request &+= 1
        let mine = request
        gate.set(mine)
        let spec = spec
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
                        do { continuation.resume(returning: try doc.applyView(spec: spec)) } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.copying.cancel()
                self.appliedSpec = spec
                self.current = view
                self.selectedDisplayRow = nil
                self.selectedSourceRow = nil
                self.isBusy = false
                self.updateFind()
            } catch {
                guard !Task.isCancelled, let self, self.request == mine else { return }
                self.isBusy = false
                self.spec = self.appliedSpec
                if case GewuError.Cancelled = error {
                    self.updateFind()
                    return
                }
                self.reportError(error)
                self.updateFind()
            }
        }
    }

    func updateFind(debounce: Bool = false, preservingMatch: Bool = false) {
        guard !isBusy else {
            find.invalidate()
            return
        }
        let columns = appliedSpec.columns.map(\.column)
        find.update(
            document: doc, version: current.version, columns: columns,
            cursor: preservingMatch ? find.currentMatch : nil, debounce: debounce)
    }

    /// Reordering columns keeps the current match instead of jumping to the first one.
    func findColumnsChanged() {
        guard find.activeQuery != nil else { return }
        updateFind(preservingMatch: true)
    }

    func stop() {
        request &+= 1
        gate.set(request)
        viewTask?.cancel()
        doc.cancelPending()
        find.stop()
        export.stop()
        copying.cancel()
        table?.stopLoading()
    }

    func findNext(backwards: Bool = false) { find.next(backwards: backwards) }
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
