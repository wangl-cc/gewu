import SwiftUI

struct DocumentScene: View {
    @StateObject private var workbook: WorkbookModel

    init(url: URL) {
        _workbook = StateObject(wrappedValue: WorkbookModel(url: url))
    }

    var body: some View {
        NavigationSplitView(columnVisibility: sidebarVisibility) {
            Group {
                if workbook.showsInspector, let model = workbook.document {
                    InspectorSidebar(model: model).id(ObjectIdentifier(model))
                } else {
                    ContentUnavailableView(
                        "Column Analysis", systemImage: "sidebar.left",
                        description: Text("Open a table to inspect its columns."))
                }
            }
            .navigationSplitViewColumnWidth(min: 220, ideal: 260, max: 360)
        } detail: {
            VStack(spacing: 0) {
                if let model = workbook.document {
                    LoadedDocumentView(model: model).id(ObjectIdentifier(model))
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
                if workbook.sheets.count > 1 {
                    Divider()
                    worksheetBar
                } else if workbook.isOpening, workbook.document != nil {
                    ProgressView("Opening worksheet…").controlSize(.small).padding(6)
                }
            }
            .navigationTitle(workbook.url.lastPathComponent)
            .navigationDocument(workbook.url)
            .navigationSubtitle(workbook.document?.subtitle ?? "")
        }
        .navigationSplitViewStyle(.balanced)
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

    private var sidebarVisibility: Binding<NavigationSplitViewVisibility> {
        Binding(
            get: { workbook.showsInspector ? .all : .detailOnly },
            set: { workbook.showsInspector = $0 != .detailOnly })
    }

    private var worksheetBar: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 4) {
                ForEach(workbook.sheets, id: \.self) { sheet in
                    Button {
                        workbook.selectSheet(sheet)
                    } label: {
                        HStack(spacing: 5) {
                            if workbook.pendingSheet == sheet { ProgressView().controlSize(.mini) }
                            Text(sheet.rawValue).lineLimit(1)
                        }
                        .padding(.horizontal, 10).padding(.vertical, 5)
                        .background {
                            if workbook.selectedSheet == sheet {
                                RoundedRectangle(cornerRadius: 5).fill(Color.accentColor.opacity(0.15))
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Worksheet \(sheet.rawValue)")
                    .accessibilityAddTraits(workbook.selectedSheet == sheet ? .isSelected : [])
                }
            }.padding(5)
        }
        .scrollIndicators(.hidden)
        .fixedSize(horizontal: false, vertical: true)
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
    @ObservedObject private var find: FindModel
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true
    @FocusState private var searchFocused: Bool
    @State private var showsSorting = false

    init(model: DocumentModel) {
        self.model = model
        _find = ObservedObject(wrappedValue: model.find)
    }

    var body: some View {
        VStack(spacing: 0) {
            DataTable(model: model, showsRowNumbers: showsRowNumbers)
            Divider()
            DocumentStatus(model: model)
        }
        .toolbar {
            ToolbarItem {
                Menu {
                    ForEach(Array(model.info.columns.enumerated()), id: \.offset) { index, column in
                        Toggle(
                            column.name,
                            isOn: Binding(
                                get: { model.spec.columns.contains { $0.column == UInt32(index) } },
                                set: {
                                    if $0 { model.showColumn(UInt32(index)) } else { model.hideColumn(UInt32(index)) }
                                }))
                    }
                } label: {
                    Label("Columns", systemImage: "tablecells")
                }
            }
            ToolbarItem {
                Button {
                    showsSorting = true
                } label: {
                    Label("Sort", systemImage: "arrow.up.arrow.down")
                }
            }
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
    @ObservedObject private var find: FindModel
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    init(model: DocumentModel) {
        self.model = model
        _find = ObservedObject(wrappedValue: model.find)
    }

    var body: some View {
        HStack(spacing: 10) {
            Text(model.status(rowNumberBase: rowNumberBase)).lineLimit(1).textSelection(.enabled)
            if model.canClearFilters {
                Button("Clear Filters") { model.clearFilters() }.buttonStyle(.borderless)
            }
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
