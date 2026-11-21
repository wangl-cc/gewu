import GewuKit
import SwiftUI

/// The content of one viewer window.
struct WindowScene: View {
    @State private var window: WindowModel

    init(url: URL) {
        _window = State(initialValue: WindowModel(url: url))
    }

    var body: some View {
        Group {
            VStack(spacing: 0) {
                if let table = window.table {
                    LoadedTableView(window: window, model: table).id(ObjectIdentifier(table))
                } else if window.isOpening {
                    ProgressView("Opening \(window.url.lastPathComponent)…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView {
                        Label("Could Not Open Table", systemImage: "exclamationmark.triangle")
                    } description: {
                        Text(window.openError ?? window.url.lastPathComponent).textSelection(.enabled)
                    } actions: {
                        Button("Try Again") { window.open() }
                        if window.supportsHeaderOption {
                            Button("Import Options…") { window.showsImportOptions = true }
                        }
                    }
                }
            }
            .navigationTitle(window.url.lastPathComponent)
            .navigationDocument(window.url)
            .navigationSubtitle(window.table.map(subtitle) ?? "")
        }
        .inspector(isPresented: $window.showsInspector) {
            Group {
                if let table = window.table {
                    InspectorSidebar(window: window, model: table).id(ObjectIdentifier(table))
                } else {
                    ContentUnavailableView(
                        "Column Analysis", systemImage: "chart.bar",
                        description: Text("Open a table to inspect its columns."))
                }
            }
            .inspectorColumnWidth(min: 260, ideal: 300, max: 460)
        }
        .frame(minWidth: 620, minHeight: 400)
        .focusedSceneValue(window)
        .task { window.open() }
        .onDisappear { window.stop() }
        .sheet(isPresented: $window.showsImportOptions) {
            ImportOptionsView(window: window)
        }
        .alert(
            window.alerts.current?.title ?? "",
            isPresented: Binding(
                get: { window.alerts.current != nil },
                set: { presented in
                    if !presented, let alert = window.alerts.current { window.alerts.dismiss(alert) }
                }),
            presenting: window.alerts.current
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { alert in
            Text(alert.message)
        }
    }

    private func subtitle(_ table: TableModel) -> String {
        let info = table.info
        let source = info.worksheet.map { "\(info.format.displayName) · \($0)" } ?? info.format.displayName
        return "\(source) · \(formatCount(info.rowCount)) rows · \(info.columns.count) columns · "
            + formatFileSize(info.fileSize)
    }
}

private struct ImportOptionsView: View {
    let window: WindowModel
    @Environment(\.dismiss) private var dismiss
    @State private var hasHeader: Bool

    init(window: WindowModel) {
        self.window = window
        _hasHeader = State(initialValue: window.hasHeader)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Import Options").font(.headline)
            Text(window.url.lastPathComponent)
                .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            Form {
                Toggle("First row contains column names", isOn: $hasHeader)
                    .help(
                        "For Excel, this means the first occupied row. The choice also applies when switching worksheets."
                    )
            }
            .formStyle(.columns)
            .toggleStyle(.checkbox)
            Text("When unchecked, all rows are data and column names are generated.")
                .font(.caption).foregroundStyle(.secondary)
            if window.table != nil {
                Text("Reopening clears search, filters and sorting.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Reopen") {
                    window.reopen(hasHeader: hasHeader)
                    dismiss()
                }.keyboardShortcut(.defaultAction)
            }
        }.padding(24).frame(width: 400)
    }
}

private struct LoadedTableView: View {
    @Bindable var window: WindowModel
    @Bindable var model: TableModel

    var body: some View {
        VStack(spacing: 0) {
            DataTable(model: model, inspectorVisible: window.showsInspector, analyze: window.inspect)
            Divider()
            StatusBar(window: window, model: model)
        }
        .toolbar {
            if !model.find.query.isEmpty {
                ToolbarItem(placement: .primaryAction) {
                    FindNavigation(model: model, find: model.find)
                }
            }
            ToolbarItem(placement: .primaryAction) {
                FindSearchField(
                    text: Bindable(model.find).query, isFocused: $window.isSearchFocused,
                    next: { model.find.next(backwards: $0) },
                    cancel: {
                        model.find.query = ""
                        model.focusTable()
                    }
                ).frame(width: 200)
            }
            if #available(macOS 26.0, *) {
                ToolbarSpacer(.fixed, placement: .primaryAction)
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    window.showsInspector.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                }.help("Show or hide column Inspector")
            }
        }
        .sheet(isPresented: $model.showsColumns) {
            VStack(spacing: 0) {
                ColumnsView(model: model)
                HStack {
                    Spacer()
                    Button("Done") { model.showsColumns = false }.keyboardShortcut(.cancelAction)
                }.padding([.horizontal, .bottom], 16)
            }
        }
        .sheet(isPresented: $model.showsSorting) {
            VStack(spacing: 0) {
                SortingView(model: model)
                HStack {
                    Spacer()
                    Button("Done") { model.showsSorting = false }.keyboardShortcut(.cancelAction)
                }.padding([.horizontal, .bottom], 20)
            }
        }
        .focusedSceneValue(model)
    }
}

private struct FindNavigation: View {
    let model: TableModel
    let find: FindModel

    var body: some View {
        HStack(spacing: 6) {
            if let status = find.status.map(Self.text) {
                Text(status).font(.caption).monospacedDigit().foregroundStyle(.secondary)
                    .lineLimit(1).frame(maxWidth: 90).help(status)
            }
            Button {
                find.next(backwards: true)
            } label: {
                Image(systemName: "chevron.up")
            }
            .disabled(!find.canNavigate).accessibilityLabel("Previous match").help("Previous match (⇧⌘G)")
            Button {
                find.next()
            } label: {
                Image(systemName: "chevron.down")
            }
            .disabled(!find.canNavigate).accessibilityLabel("Next match").help("Next match (⌘G)")
        }
        .buttonStyle(.borderless).controlSize(.small)
    }

    static func text(_ status: FindModel.Status) -> String {
        switch status {
        case .finding: "Finding…"
        case .noMatches: "0 matches"
        case .ready: "Press Return"
        case .match(let row): "Row \(formatCount(row + 1))"
        }
    }
}

private struct StatusBar: View {
    let window: WindowModel
    let model: TableModel
    @State private var showsFilters = false
    @State private var showsSorting = false

    var body: some View {
        HStack(spacing: 10) {
            if window.sheets.count > 1 {
                Menu {
                    ForEach(window.sheets, id: \.self) { sheet in
                        Button {
                            window.selectSheet(sheet)
                        } label: {
                            if window.selectedSheet == sheet {
                                Label(sheet.rawValue, systemImage: "checkmark")
                            } else {
                                Text(sheet.rawValue)
                            }
                        }
                    }
                } label: {
                    Label(window.selectedSheet?.rawValue ?? "Worksheets", systemImage: "tablecells")
                        .lineLimit(1)
                }.frame(maxWidth: 180).help("Switch worksheet")
            }
            if window.isOpening {
                ProgressView().controlSize(.small)
                    .help(window.pendingSheet.map { "Opening \($0.rawValue)…" } ?? "Opening…")
            }
            Text(rowSummary(model)).monospacedDigit().lineLimit(1).textSelection(.enabled)
            let filtered = model.filteredColumns.count
            if filtered > 0 {
                Text("·")
                Button {
                    showsFilters.toggle()
                } label: {
                    Text("\(filtered) filtered").monospacedDigit().fixedSize()
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("\(filtered) filtered columns")
                .help("Review active column filters")
                .popover(isPresented: $showsFilters, arrowEdge: .bottom) {
                    ActiveFiltersView(model: model) { column in
                        showsFilters = false
                        model.select(column)
                        model.revealColumn(column, focusingTable: true)
                    }
                }
            }
            let sorted = model.appliedQuery.sorts.count
            if sorted > 0 {
                Text("·")
                Button {
                    showsSorting.toggle()
                } label: {
                    Text("\(sorted) sorted").monospacedDigit().fixedSize()
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("\(sorted) sorted columns")
                .help("Review sort order and priority")
                .popover(isPresented: $showsSorting, arrowEdge: .bottom) { SortingView(model: model) }
            }
            Spacer(minLength: 0)
            CopyStatus(copying: model.copying)
            ExportStatus(export: model.export)
            if model.isUpdating { ProgressView().controlSize(.small).help("Updating view") }
        }
        .font(.caption).foregroundStyle(.secondary)
        .padding(.horizontal, 12).frame(height: 28)
        .background(Color(nsColor: .windowBackgroundColor))
        .onChange(of: model.filteredColumns.isEmpty) { _, empty in
            if empty { showsFilters = false }
        }
    }
}

/// The status bar's description of the displayed rows. A pending query does
/// not change it until its rows show.
@MainActor
func rowSummary(_ model: TableModel) -> String {
    let rows = UInt64(model.displayed.count)
    let total = model.info.rowCount
    let text =
        model.filteredColumns.isEmpty && rows == total
        ? "\(formatCount(rows)) rows" : "\(formatCount(rows)) / \(formatCount(total)) rows"
    return text + " · \(model.visibleColumns.count) columns"
}

private struct ActiveFiltersView: View {
    let model: TableModel
    let locate: (ColumnID) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Active Filters").font(.headline)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(model.appliedQuery.filters, id: \.column) { entry in
                        let name = model.column(entry.column)?.name ?? ""
                        HStack(spacing: 12) {
                            Button {
                                locate(entry.column)
                            } label: {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(name).fontWeight(.medium).foregroundStyle(.primary)
                                    Text(entry.filter.detailSummary)
                                        .font(.caption).foregroundStyle(.secondary).lineLimit(3)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .help("Show \(name) in the table\n\(entry.filter.detailSummary)")
                            Button {
                                model.setFilter(nil, column: entry.column)
                            } label: {
                                Image(systemName: "xmark").foregroundStyle(.secondary)
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel("Clear filter on \(name)")
                            .help("Clear filter on \(name)")
                            .disabled(model.isUpdating)
                        }.padding(.vertical, 8)
                    }
                }
            }
            .frame(maxHeight: 280)
            .fixedSize(horizontal: false, vertical: true)
            Divider()
            Button("Clear All Filters") { model.clearFilters() }
                .buttonStyle(.borderless).disabled(model.isUpdating)
        }
        .controlSize(.small).padding(16).frame(width: 340)
    }
}

private struct CopyStatus: View {
    let copying: CopyModel

    var body: some View {
        switch copying.outcome {
        case .copied: Text("Copied selection").lineLimit(1)
        case .tooLarge: Text("Selection is too large to copy; use File > Export…").lineLimit(1)
        case nil: EmptyView()
        }
    }
}

private struct ExportStatus: View {
    let export: ExportModel

    var body: some View {
        if let progress = export.progress {
            ProgressView(value: Double(progress.rowsWritten), total: Double(max(progress.totalRows, 1)))
                .frame(width: 70)
            Text("Exporting \(formatCount(progress.rowsWritten)) of \(formatCount(progress.totalRows))")
                .monospacedDigit()
            Button("Cancel") { export.cancel() }.buttonStyle(.borderless)
        } else {
            switch export.outcome {
            case .exported(let name): Text("Exported \(name)").lineLimit(1).help("Exported \(name)")
            case .cancelled: Text("Export cancelled").lineLimit(1)
            case nil: EmptyView()
            }
        }
    }
}
