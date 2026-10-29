import SwiftUI

@main
struct GewuApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var files = FileCoordinator.shared

    var body: some Scene {
        WindowGroup("Gewu", id: "viewer", for: URL.self) { $url in
            ViewerScene(url: url).environmentObject(files)
        }
        .defaultSize(width: 1200, height: 720)
        .windowToolbarStyle(.unified)
        .commands { ViewerCommands(files: files) }

        WindowGroup("File Info", id: "file-info", for: FileDetails.self) { $details in
            if let details { FileInfoView(details: details) }
        }
        .defaultSize(width: 460, height: 440)
        .windowResizability(.contentMinSize)
        .restorationBehavior(.disabled)
    }
}

private struct ViewerScene: View {
    let url: URL?
    @EnvironmentObject private var files: FileCoordinator
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if let url { DocumentScene(url: url).id(url) }
            else { WelcomeView() }
        }
        .onAppear {
            files.connect(openWindow: { openWindow(id: "viewer", value: $0) },
                          showWelcome: { openWindow(id: "viewer") })
        }
        .onOpenURL { files.open($0) }
    }
}

private struct WelcomeView: View {
    @EnvironmentObject private var files: FileCoordinator
    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "tablecells").font(.system(size: 48)).foregroundStyle(.secondary)
            Text("Gewu 格物").font(.largeTitle)
            Text("Open a Parquet, CSV, TSV or XLSX file to explore its data.").foregroundStyle(.secondary)
            Button("Open…", systemImage: "folder") { files.chooseFiles() }.buttonStyle(.borderedProminent)
            if !files.recentURLs.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Recent Files").font(.headline)
                    ForEach(Array(files.recentURLs.prefix(8)), id: \.self) { url in
                        Button { files.open(url) } label: {
                            Label(url.lastPathComponent, systemImage: "doc")
                                .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.plain).help(url.path)
                    }
                }.frame(maxWidth: 420)
            }
        }
        .padding(40)
        .frame(minWidth: 620, maxWidth: .infinity, minHeight: 400, maxHeight: .infinity)
    }
}

private struct DocumentScene: View {
    @StateObject private var model: DocumentModel
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true
    @FocusState private var searchFocused: Bool

    init(url: URL) { _model = StateObject(wrappedValue: DocumentModel(url: url)) }

    var body: some View {
        VStack(spacing: 0) {
            if model.doc != nil {
                DataTable(model: model, showsRowNumbers: showsRowNumbers)
            } else if model.isBusy {
                ProgressView("Opening \(model.url.lastPathComponent)…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView {
                    Label("Could Not Open File", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(model.errorMessage ?? model.url.lastPathComponent)
                } actions: {
                    Button("Try Again") { Task { await model.open() } }
                }
            }
            Divider()
            HStack(spacing: 10) {
                Text(model.status).lineLimit(1).textSelection(.enabled)
                if !model.columnFilters.isEmpty {
                    Button("Clear Filters") { model.clearFilters() }.buttonStyle(.borderless)
                }
                Spacer(minLength: 0)
                if let progress = model.exportProgress {
                    ProgressView(value: Double(progress.rowsWritten), total: Double(max(progress.totalRows, 1)))
                        .frame(width: 70)
                    Text("Exporting \(formatCount(progress.rowsWritten)) of \(formatCount(progress.totalRows))")
                        .monospacedDigit()
                    Button("Cancel") { model.cancelExport() }.buttonStyle(.borderless)
                } else if let message = model.exportMessage {
                    Text(message).lineLimit(1).help(message)
                }
                if model.isBusy { ProgressView().controlSize(.small).help("Updating view") }
            }
            .font(.caption).foregroundStyle(.secondary)
            .padding(.horizontal, 12).frame(height: 28)
        }
        .frame(minWidth: 620, minHeight: 400)
        .navigationTitle(model.url.lastPathComponent)
        .navigationDocument(model.url)
        .navigationSubtitle(model.subtitle)
        .searchable(text: $model.searchText, placement: .toolbar, prompt: "Search all columns")
        .searchFocused($searchFocused)
        .onSubmit(of: .search) { model.findNext(backwards: NSEvent.modifierFlags.contains(.shift)) }
        .focusedSceneObject(model)
        .onChange(of: model.searchText) { _, _ in model.refresh(debounce: true) }
        .onChange(of: model.searchFocusRequest) { _, _ in searchFocused = true }
        .task { await model.open() }
        .onDisappear { model.stop() }
        .alert("Gewu", isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })) {
            Button("OK", role: .cancel) { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
}

private struct ViewerCommands: Commands {
    @ObservedObject var files: FileCoordinator
    @FocusedObject private var model: DocumentModel?
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Open…") { files.chooseFiles() }.keyboardShortcut("o")
            Menu("Open Recent") {
                ForEach(files.recentURLs, id: \.self) { url in
                    Button(url.lastPathComponent) { files.open(url) }.help(url.path)
                }
                Divider()
                Button("Clear Menu") { files.clearRecent() }.disabled(files.recentURLs.isEmpty)
            }
            Button("Show Welcome Window") { openWindow(id: "viewer") }
            Divider()
            Button("Get Info…") {
                if let info = model?.info { openWindow(id: "file-info", value: FileDetails(info: info)) }
            }.keyboardShortcut("i").disabled(model?.info == nil)
        }
        CommandGroup(after: .newItem) {
            Divider()
            Button("Export…") { model?.chooseExport() }
                .keyboardShortcut("e", modifiers: [.command, .shift])
                .disabled(model?.canExport != true)
        }
        CommandGroup(after: .textEditing) {
            Divider()
            Button("Find…") { model?.focusSearch() }.keyboardShortcut("f").disabled(model?.doc == nil)
            Button("Find Next") { model?.findNext() }.keyboardShortcut("g")
                .disabled(model?.appliedState.search == nil || model?.current.rowCount == 0)
            Button("Find Previous") { model?.findNext(backwards: true) }.keyboardShortcut("g", modifiers: [.command, .shift])
                .disabled(model?.appliedState.search == nil || model?.current.rowCount == 0)
        }
        CommandGroup(after: .sidebar) {
            Button("Column Info…") { model?.table?.showSelectedColumnInfo() }
                .keyboardShortcut("i", modifiers: [.command, .option])
                .disabled(model?.doc == nil || model?.selectedColumn == nil || model?.isBusy == true)
            Toggle("Show Row Numbers", isOn: $showsRowNumbers).keyboardShortcut("r", modifiers: [.command, .option])
                .disabled(model == nil)
        }
        CommandMenu("Filter") {
            Button("Filter Column…") { model?.table?.showSelectedColumnFilter() }
                .keyboardShortcut("f", modifiers: [.command, .option])
                .disabled(model?.doc == nil || model?.selectedColumn == nil)
            Divider()
            Button("Clear All Filters") { model?.clearFilters() }.keyboardShortcut("l", modifiers: [.command, .shift])
                .disabled(model?.columnFilters.isEmpty != false)
        }
    }
}
