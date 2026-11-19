import SwiftUI

struct DocumentScene: View {
    @StateObject private var workbook: WorkbookModel

    init(url: URL) {
        _workbook = StateObject(wrappedValue: WorkbookModel(url: url))
    }

    var body: some View {
        Group {
            VStack(spacing: 0) {
                if let model = workbook.document {
                    LoadedDocumentView(model: model, workbook: workbook).id(ObjectIdentifier(model))
                } else if workbook.isOpening {
                    ProgressView("Opening \(workbook.url.lastPathComponent)…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView {
                        Label("Could Not Open Table", systemImage: "exclamationmark.triangle")
                    } description: {
                        Text(workbook.errorMessage ?? workbook.url.lastPathComponent).textSelection(.enabled)
                    } actions: {
                        Button("Try Again") { workbook.open() }
                        if workbook.supportsHeaderOption {
                            Button("Import Options…") { workbook.showsImportOptions = true }
                        }
                    }
                }
            }
            .navigationTitle(workbook.url.lastPathComponent)
            .navigationDocument(workbook.url)
            .navigationSubtitle(workbook.document?.subtitle ?? "")
        }
        .inspector(isPresented: $workbook.showsInspector) {
            Group {
                if let model = workbook.document {
                    InspectorSidebar(model: model).id(ObjectIdentifier(model))
                } else {
                    ContentUnavailableView(
                        "Column Analysis", systemImage: "chart.bar",
                        description: Text("Open a table to inspect its columns."))
                }
            }
            .inspectorColumnWidth(min: 260, ideal: 300, max: 460)
        }
        .frame(minWidth: 620, minHeight: 400)
        .focusedSceneObject(workbook)
        .task { workbook.open() }
        .onDisappear { workbook.stop() }
        .sheet(isPresented: $workbook.showsImportOptions) {
            ImportOptionsView(workbook: workbook)
        }
        .alert("Could Not Open Table", isPresented: $workbook.isErrorPresented) {
            Button("OK", role: .cancel) { workbook.isErrorPresented = false }
        } message: {
            Text(workbook.errorMessage ?? "")
        }
    }

}

private struct ImportOptionsView: View {
    @ObservedObject var workbook: WorkbookModel
    @Environment(\.dismiss) private var dismiss
    @State private var hasHeader: Bool

    init(workbook: WorkbookModel) {
        self.workbook = workbook
        _hasHeader = State(initialValue: workbook.hasHeader)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Import Options").font(.headline)
            Text(workbook.url.lastPathComponent)
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
            if workbook.document != nil {
                Text("Reopening clears search, filters and sorting.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Reopen") {
                    workbook.reopen(hasHeader: hasHeader)
                    dismiss()
                }.keyboardShortcut(.defaultAction)
            }
        }.padding(24).frame(width: 400)
    }
}

private struct LoadedDocumentView: View {
    @Bindable var model: DocumentModel
    @ObservedObject var workbook: WorkbookModel
    @ObservedObject private var find: FindModel
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true

    init(model: DocumentModel, workbook: WorkbookModel) {
        self.model = model
        self.workbook = workbook
        _find = ObservedObject(wrappedValue: model.find)
    }

    var body: some View {
        VStack(spacing: 0) {
            DataTable(model: model, showsRowNumbers: showsRowNumbers)
            Divider()
            DocumentStatus(model: model, workbook: workbook)
        }
        .toolbar {
            if !find.query.isEmpty {
                ToolbarItem(placement: .primaryAction) {
                    findNavigation
                }
            }
            searchItem
            if #available(macOS 26.0, *) {
                ToolbarSpacer(.fixed, placement: .primaryAction)
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    workbook.showsInspector.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                }.help("Show or hide column Inspector")
            }
        }
        .onChange(of: model.inspectorRequest) { _, _ in workbook.showsInspector = true }
        .onChange(of: workbook.showsInspector, initial: true) { _, visible in
            model.inspectorVisible = visible
        }
        .onChange(of: find.query) { _, _ in model.updateFind(debounce: true) }
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
        .alert("Gewu", isPresented: $model.isErrorPresented) {
            Button("OK", role: .cancel) { model.isErrorPresented = false }
        } message: {
            Text(model.errorMessage ?? "")
        }
    }

    private var searchItem: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            FindSearchField(
                text: $find.query, focusRequest: model.searchFocusRequest,
                next: { model.findNext(backwards: $0) },
                cancel: {
                    find.query = ""
                    model.table?.focusTable()
                }
            ).frame(width: 200)
        }
    }

    private var findNavigation: some View {
        HStack(spacing: 6) {
            if let status = find.status {
                Text(status).font(.caption).monospacedDigit().foregroundStyle(.secondary)
                    .lineLimit(1).frame(maxWidth: 90).help(status)
            }
            Button {
                model.findNext(backwards: true)
            } label: {
                Image(systemName: "chevron.up")
            }
            .disabled(!find.canNavigate).accessibilityLabel("Previous match").help("Previous match (⇧⌘G)")
            Button {
                model.findNext()
            } label: {
                Image(systemName: "chevron.down")
            }
            .disabled(!find.canNavigate).accessibilityLabel("Next match").help("Next match (⌘G)")
        }
        .buttonStyle(.borderless).controlSize(.small)
    }
}

private struct DocumentStatus: View {
    let model: DocumentModel
    @ObservedObject var workbook: WorkbookModel
    @State private var showsFilters = false
    @State private var showsSorting = false

    init(model: DocumentModel, workbook: WorkbookModel) {
        self.model = model
        self.workbook = workbook
    }

    var body: some View {
        HStack(spacing: 10) {
            if workbook.sheets.count > 1 {
                Menu {
                    ForEach(workbook.sheets, id: \.self) { sheet in
                        Button {
                            workbook.selectSheet(sheet)
                        } label: {
                            if workbook.selectedSheet == sheet {
                                Label(sheet.rawValue, systemImage: "checkmark")
                            } else {
                                Text(sheet.rawValue)
                            }
                        }
                    }
                } label: {
                    Label(workbook.selectedSheet?.rawValue ?? "Worksheets", systemImage: "tablecells")
                        .lineLimit(1)
                }.frame(maxWidth: 180).help("Switch worksheet")
            }
            if workbook.isOpening {
                ProgressView().controlSize(.small)
                    .help(workbook.pendingSheet.map { "Opening \($0.rawValue)…" } ?? "Opening…")
            }
            Text(model.status).monospacedDigit().lineLimit(1).textSelection(.enabled)
            if !model.appliedFilterColumns.isEmpty {
                Text("·")
                Button {
                    showsFilters.toggle()
                } label: {
                    Text("\(model.appliedFilterColumns.count) filtered")
                        .monospacedDigit().fixedSize()
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("\(model.appliedFilterColumns.count) filtered columns")
                .help("Review active column filters")
                .popover(isPresented: $showsFilters, arrowEdge: .bottom) {
                    ActiveFiltersView(model: model) { column in
                        showsFilters = false
                        model.selectedColumn = column
                        model.table?.scrollToColumn(column, focus: true)
                    }
                }
            }
            let sortCount = model.viewer.appliedSorts.count
            if sortCount > 0 {
                Text("·")
                Button {
                    showsSorting.toggle()
                } label: {
                    Text("\(sortCount) sorted").monospacedDigit().fixedSize()
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("\(sortCount) sorted columns")
                .help("Review sort order and priority")
                .popover(isPresented: $showsSorting, arrowEdge: .bottom) { SortingView(model: model) }
            }
            Spacer(minLength: 0)
            CopyStatus(copying: model.copying)
            ExportStatus(export: model.export)
            if model.isBusy { ProgressView().controlSize(.small).help("Updating view") }
        }
        .font(.caption).foregroundStyle(.secondary)
        .padding(.horizontal, 12).frame(height: 28)
        .background(Color(nsColor: .windowBackgroundColor))
        .onChange(of: model.appliedFilterColumns.isEmpty) { _, empty in
            if empty { showsFilters = false }
        }
    }
}

private struct ActiveFiltersView: View {
    let model: DocumentModel
    let locate: (ColumnId) -> Void

    var body: some View {
        let columns = model.viewer.appliedFilters
        VStack(alignment: .leading, spacing: 12) {
            Text("Active Filters").font(.headline)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(columns, id: \.column) { entry in
                        let name = model.columnInfo[entry.column]?.name ?? ""
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
                            .disabled(model.isBusy)
                        }.padding(.vertical, 8)
                    }
                }
            }
            .frame(maxHeight: 280)
            .fixedSize(horizontal: false, vertical: true)
            Divider()
            Button("Clear All Filters") { model.clearFilters() }
                .buttonStyle(.borderless).disabled(model.isBusy)
        }
        .controlSize(.small).padding(16).frame(width: 340)
    }
}

private struct CopyStatus: View {
    @ObservedObject var copying: CopyModel

    var body: some View {
        if copying.isCopying {
            ProgressView().controlSize(.small)
            Text("Copying selection…")
            Button("Cancel Copy") { copying.cancel() }.buttonStyle(.borderless)
        } else if let message = copying.message {
            Text(message).lineLimit(1)
        }
    }
}

private struct ExportStatus: View {
    @ObservedObject var export: ExportModel

    var body: some View {
        if let progress = export.progress {
            ProgressView(value: Double(progress.rowsWritten), total: Double(max(progress.totalRows, 1)))
                .frame(width: 70)
            Text("Exporting \(formatCount(progress.rowsWritten)) of \(formatCount(progress.totalRows))")
                .monospacedDigit()
            Button("Cancel") { export.cancel() }.buttonStyle(.borderless)
        } else if let message = export.message {
            Text(message).lineLimit(1).help(message)
        }
    }
}
