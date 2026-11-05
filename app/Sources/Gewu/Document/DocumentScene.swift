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
            .inspectorColumnWidth(min: 240, ideal: 300, max: 460)
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    workbook.showsInspector.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                }.help("Show or hide Inspector")
            }
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
    @ObservedObject var model: DocumentModel
    @ObservedObject var workbook: WorkbookModel
    @ObservedObject private var find: FindModel
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true
    @FocusState private var searchFocused: Bool
    @State private var showsSorting = false

    init(model: DocumentModel, workbook: WorkbookModel) {
        self.model = model
        self.workbook = workbook
        _find = ObservedObject(wrappedValue: model.find)
    }

    var body: some View {
        VStack(spacing: 0) {
            AppliedFiltersView(model: model)
            DataTable(model: model, showsRowNumbers: showsRowNumbers)
            Divider()
            DocumentStatus(model: model, workbook: workbook)
        }
        .toolbar {
            ToolbarItem {
                Button {
                    model.showsColumns.toggle()
                } label: {
                    Label("Columns", systemImage: "tablecells")
                }
                .popover(isPresented: $model.showsColumns) { ColumnsView(model: model) }
            }
            ToolbarItem {
                ExportToolbarButton(model: model, export: model.export)
            }
            ToolbarItem {
                Button {
                    showsSorting = true
                } label: {
                    Label("Sort", systemImage: "arrow.up.arrow.down")
                }
            }
        }
        .onChange(of: model.inspectorRequest) { _, _ in workbook.showsInspector = true }
        .onChange(of: workbook.showsInspector, initial: true) { _, visible in
            model.inspectorVisible = visible
        }
        .sheet(isPresented: $showsSorting) { SortingView(model: model) }
        .searchable(text: $find.query, placement: .toolbar, prompt: "Find in visible columns")
        .searchFocused($searchFocused)
        .onSubmit(of: .search) { model.findNext(backwards: NSEvent.modifierFlags.contains(.shift)) }
        .onChange(of: find.query) { _, _ in model.updateFind(debounce: true) }
        .onChange(of: model.searchFocusRequest) { _, _ in searchFocused = true }
        .focusedSceneObject(model)
        .alert("Gewu", isPresented: $model.isErrorPresented) {
            Button("OK", role: .cancel) { model.isErrorPresented = false }
        } message: {
            Text(model.errorMessage ?? "")
        }
    }
}

private struct DocumentStatus: View {
    @ObservedObject var model: DocumentModel
    @ObservedObject var workbook: WorkbookModel
    @ObservedObject private var find: FindModel

    init(model: DocumentModel, workbook: WorkbookModel) {
        self.model = model
        self.workbook = workbook
        _find = ObservedObject(wrappedValue: model.find)
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
            Spacer(minLength: 0)
            CopyStatus(copying: model.copying)
            ExportStatus(export: model.export)
            if model.isBusy { ProgressView().controlSize(.small).help("Updating view") }
        }
        .font(.caption).foregroundStyle(.secondary)
        .padding(.horizontal, 12).frame(height: 28)
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

private struct SortingView: View {
    @ObservedObject var model: DocumentModel
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sort Columns").font(.headline)
            Text("Top to bottom priority. Shift-click a header to add a key; click again for descending, then no sort.")
                .font(.caption).foregroundStyle(.secondary)
            List {
                ForEach(Array(model.sortedColumns.enumerated()), id: \.element.column) { index, col in
                    HStack {
                        Text("\(index + 1). \(model.info.columns[Int(col.column)].name)")
                        Spacer()
                        Picker(
                            "Direction",
                            selection: Binding(
                                get: { col.sort?.descending ?? false },
                                set: { model.setSortDirection(column: col.column, descending: $0) })
                        ) {
                            Text("Ascending").tag(false)
                            Text("Descending").tag(true)
                        }.labelsHidden().frame(width: 115)
                        Button {
                            move(index, by: -1)
                        } label: {
                            Image(systemName: "arrow.up")
                        }.disabled(index == 0)
                        Button {
                            move(index, by: 1)
                        } label: {
                            Image(systemName: "arrow.down")
                        }.disabled(index == model.sortedColumns.count - 1)
                        Button("Remove") { model.removeSort(column: col.column) }
                    }
                }.onMove { from, to in
                    var columns = model.sortedColumns.map(\.column)
                    columns.move(fromOffsets: from, toOffset: to)
                    model.reorderSort(columns)
                }
            }.frame(height: 210)
            HStack {
                Menu("Add Column") {
                    ForEach(model.spec.columns.filter { $0.sort == nil }, id: \.column) { col in
                        Button(model.info.columns[Int(col.column)].name) {
                            model.setSortDirection(column: col.column, descending: false)
                        }
                    }
                }.disabled(model.spec.columns.allSatisfy { $0.sort != nil })
                Button("Clear All") { model.clearSort() }.disabled(model.sortedColumns.isEmpty)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(width: 560)
    }
    private func move(_ index: Int, by delta: Int) {
        var columns = model.sortedColumns.map(\.column)
        columns.swapAt(index, index + delta)
        model.reorderSort(columns)
    }
}
