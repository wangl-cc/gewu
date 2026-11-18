import Combine
import Foundation

enum InspectorPage: String, CaseIterable {
    case column = "Column"
    case chart = "Chart"
}

/// One loaded table's presentation state. Rust owns the data; SwiftUI observes only
/// metadata and immutable views, never an array containing the whole table.
@MainActor
final class DocumentModel: ObservableObject {
    let url: URL
    let dataset: GewuDataset
    let info: DatasetInfo
    let kinds: [ValueCategory]
    let find: FindModel
    /// The applied View, its spec and the results computed for it.
    let viewer: Viewer
    var applied: Viewer.Applied { viewer.applied }
    var view: GewuView { applied.view }
    var appliedSpec: QuerySpec { applied.spec }
    var isBusy: Bool { submittedSpec != nil }
    @Published private(set) var errorMessage: String?
    @Published var isErrorPresented = false
    @Published var selectedColumn: Int? { didSet { publishTable() } }
    @Published var inspectedColumn: Int? { didSet { publishTable() } }
    @Published var inspectorPage = InspectorPage.column
    @Published var showsSorting = false
    @Published var inspectorRequest = 0
    @Published var showsColumns = false
    @Published var inspectorVisible = true { didSet { publishTable() } }
    let inspectorSettings = InspectorSettings()
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
    private(set) lazy var tablePresentation = TablePresentation(tableInput)
    private var viewTask: Task<Void, Never>?
    @Published private var submittedSpec: QuerySpec?

    private var tableInput: TablePresentation.Input {
        .init(
            rows: applied.rowView, spec: appliedSpec, selectedColumn: selectedColumn,
            inspectedColumn: inspectorVisible ? inspectedColumn : nil)
    }

    private func publishTable() { tablePresentation.apply(tableInput) }

    /// Receives the initial View already allocated by the background loading worker.
    init(url: URL, dataset: GewuDataset, view: GewuView) {
        self.url = url
        self.dataset = dataset
        let metadata = dataset.info()
        info = metadata
        kinds = metadata.columns.map(\.valueCategory)
        find = FindModel()
        let initial = QuerySpec(
            columns: metadata.columns.indices.map {
                ColSpec(column: UInt32($0), sort: nil, filter: nil)
            })
        viewer = Viewer(dataset: dataset, view: view, spec: initial)
        spec = initial
        selectedColumn = metadata.columns.isEmpty ? nil : 0
        inspectedColumn = selectedColumn
        find.onNavigate = { [weak self] match in self?.table?.navigateToMatch(match) }
        find.onError = { [weak self] error, prefix in self?.reportError(error, prefix: prefix) }
    }

    var canExport: Bool { !isBusy && export.isIdle && !appliedSpec.columns.isEmpty }

    var subtitle: String {
        let source =
            switch info.metadata {
            case .xlsx(let sheet, _): "\(info.format.displayName) · \(sheet)"
            default: info.format.displayName
            }
        return "\(source) · \(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
            + ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file)
    }

    var status: String {
        let text =
            appliedFilterColumns.isEmpty && view.rowCount() == info.rowCount
            ? "\(formatCount(view.rowCount())) rows"
            : "\(formatCount(view.rowCount())) / \(formatCount(info.rowCount)) rows"
        return text + " · \(appliedSpec.columns.count) columns"
    }

    var sortedColumns: [ColSpec] {
        spec.sortedIndices.map { spec.columns[$0] }
    }

    private func submit(_ next: QuerySpec) {
        guard next != spec else { return }
        spec = next
        refresh()
    }

    func cycleSort(column: UInt32, multiple: Bool) {
        guard let index = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        var next = spec
        let old = spec.columns[index].sort
        if !multiple { for i in next.columns.indices { next.columns[i].sort = nil } }
        if old?.descending == true {
            next.columns[index].sort = nil
        } else {
            let priority = multiple ? (old?.priority ?? UInt32(sortedColumns.count)) : 0
            next.columns[index].sort = ColSort(priority: priority, descending: old != nil)
        }
        next.normalizeSort()
        submit(next)
    }

    func setSortDirection(column: UInt32, descending: Bool) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        var next = spec
        next.columns[i].sort = ColSort(
            priority: spec.columns[i].sort?.priority ?? UInt32(sortedColumns.count), descending: descending)
        submit(next)
    }

    func removeSort(column: UInt32) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        var next = spec
        next.columns[i].sort = nil
        next.normalizeSort()
        submit(next)
    }

    func clearSort() {
        var next = spec
        for i in next.columns.indices { next.columns[i].sort = nil }
        submit(next)
    }

    func replaceSortColumn(_ column: UInt32, with replacement: UInt32) {
        guard column != replacement,
            let source = spec.columns.firstIndex(where: { $0.column == column }),
            let destination = spec.columns.firstIndex(where: { $0.column == replacement }),
            let sort = spec.columns[source].sort,
            spec.columns[destination].sort == nil
        else { return }
        var next = spec
        next.columns[destination].sort = sort
        next.columns[source].sort = nil
        submit(next)
    }

    func reorderSort(_ columns: [UInt32]) {
        guard Set(columns) == Set(sortedColumns.map(\.column)), columns.count == sortedColumns.count else { return }
        var next = spec
        for (priority, column) in columns.enumerated() {
            if let i = spec.columns.firstIndex(where: { $0.column == column }) {
                next.columns[i].sort?.priority = UInt32(priority)
            }
        }
        submit(next)
    }

    func setFilter(_ filter: Filter?, column: UInt32) {
        guard let i = spec.columns.firstIndex(where: { $0.column == column }) else { return }
        var next = spec
        next.columns[i].filter = filter
        submit(next)
    }

    var canClearFilters: Bool { spec.columns.contains { $0.filter != nil } || !appliedFilterColumns.isEmpty }

    func clearFilters() {
        guard canClearFilters else { return }
        var next = spec
        for i in next.columns.indices { next.columns[i].filter = nil }
        submit(next)
    }

    func reorderColumns(_ columns: [UInt32]) {
        guard columns != spec.columns.map(\.column), Set(columns) == Set(spec.columns.map(\.column)),
            columns.count == spec.columns.count
        else { return }
        var next = spec
        let byColumn = Dictionary(uniqueKeysWithValues: spec.columns.map { ($0.column, $0) })
        next.columns = columns.compactMap { byColumn[$0] }
        if !isBusy && spec == appliedSpec {
            do {
                try viewer.reorder(columns, spec: next)
                spec = next
                publishTable()
                updateFind(preservingMatch: true)
            } catch {
                reportError(error)
            }
        } else {
            submit(next)
        }
    }

    /// Apply a hide operation after any required confirmation.
    func removeColumn(_ column: UInt32) {
        setColumnsVisible([column], visible: false)
    }

    func showColumn(_ column: UInt32) {
        setColumnsVisible([column], visible: true)
    }

    func showAllColumns() {
        setColumnsVisible(info.columns.indices.map(UInt32.init), visible: true)
    }

    /// Applies a confirmed bulk visibility edit as one query, preserving surviving display order.
    func setColumnsVisible(_ columns: [UInt32], visible: Bool) {
        let targets = Set(columns.filter { Int($0) < info.columns.count })
        var next = spec
        if visible {
            let shown = Set(next.columns.map(\.column))
            next.columns += targets.subtracting(shown).sorted().map { ColSpec(column: $0, sort: nil, filter: nil) }
        } else {
            next.columns.removeAll { targets.contains($0.column) }
            next.normalizeSort()
            if let selectedColumn, targets.contains(UInt32(selectedColumn)) {
                self.selectedColumn = next.columns.first.map { Int($0.column) }
            }
        }
        submit(next)
    }

    func inspectColumn(_ column: Int) {
        guard info.columns.indices.contains(column) else { return }
        inspectedColumn = column
        inspectorPage = .column
        inspectorRequest &+= 1
    }

    func refresh(debounce: Bool = false) {
        guard spec != submittedSpec else { return }
        viewTask?.cancel()
        if spec == appliedSpec {
            viewTask = nil
            if isBusy {
                submittedSpec = nil
                updateFind()
            }
            return
        }
        find.invalidate()
        let spec = spec
        submittedSpec = spec
        let viewer = viewer
        viewTask = Task { [weak self] in
            do {
                let view = try await viewer.query(spec, debounce: debounce)
                guard !Task.isCancelled, let self else { return }
                var prepared: TableRowLoader.Snapshot?
                while let table = self.table {
                    prepared = try await table.loadViewport(in: view, spec: spec)
                    try Task.checkCancellation()
                    if self.table === table { break }
                }
                try Task.checkCancellation()
                self.commit(view, spec: spec, rows: prepared)
            } catch {
                guard !Task.isCancelled, let self else { return }
                self.viewTask = nil
                self.submittedSpec = nil
                self.spec = self.appliedSpec
                if !error.isCancellation { self.reportError(error) }
                self.updateFind()
            }
        }
    }

    /// Publish the View and its ready cells together, without suspending or
    /// waiting for SwiftUI to deliver another table update.
    private func commit(_ view: GewuView, spec: QuerySpec, rows: TableRowLoader.Snapshot?) {
        objectWillChange.send()
        viewer.apply(view, spec: spec)
        publishTable()
        if let rows { table?.commit(rows) }
        copying.cancel()
        selectedDisplayRow = nil
        selectedSourceRow = nil
        viewTask = nil
        submittedSpec = nil
        updateFind()
    }

    func updateFind(debounce: Bool = false, preservingMatch: Bool = false) {
        guard !isBusy else {
            find.invalidate()
            return
        }
        let columns = appliedSpec.columns.map(\.column)
        find.update(
            view: view, columns: columns,
            cursor: preservingMatch ? find.currentMatch : nil, debounce: debounce)
    }

    func stop() {
        viewTask?.cancel()
        viewTask = nil
        submittedSpec = nil
        viewer.close()
        spec = appliedSpec
        find.stop()
        export.stop()
        copying.cancel()
        table?.stopLoading()
    }

    func findNext(backwards: Bool = false) {
        find.next(backwards: backwards)
    }
    func focusSearch() {
        searchFocusRequest &+= 1
    }

    private func reportError(_ message: String) {
        errorMessage = message
        isErrorPresented = true
    }

    func reportError(_ error: Error, prefix: String = "") {
        reportError(prefix + error.userMessage)
    }
}

extension QuerySpec {
    fileprivate var sortedIndices: [Int] {
        columns.indices.filter { columns[$0].sort != nil }
            .sorted { columns[$0].sort!.priority < columns[$1].sort!.priority }
    }

    fileprivate mutating func normalizeSort() {
        for (priority, index) in sortedIndices.enumerated() {
            columns[index].sort?.priority = UInt32(priority)
        }
    }
}
