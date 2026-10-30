import SwiftUI

struct ViewerCommands: Commands {
    @ObservedObject var files: FileCoordinator
    @FocusedObject private var model: DocumentModel?
    @FocusedObject private var workbook: WorkbookModel?
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
            Divider()
            Button("Import Options…") { workbook?.showsImportOptions = true }
                .disabled(workbook?.supportsHeaderOption != true || workbook?.isOpening == true)
            Button("Get Info…") {
                if let info = model?.info { openWindow(id: "file-info", value: FileDetails(info: info)) }
            }.keyboardShortcut("i").disabled(model?.info == nil)
        }
        CommandGroup(after: .newItem) {
            Divider()
            if let model {
                ExportCommand(model: model, export: model.export)
            } else {
                Button("Export…") {}.keyboardShortcut("e", modifiers: [.command, .shift]).disabled(true)
            }
        }
        CommandGroup(after: .textEditing) {
            Divider()
            Button("Find…") { model?.focusSearch() }.keyboardShortcut("f").disabled(model?.doc == nil)
            Button("Find Next") { model?.findNext() }.keyboardShortcut("g")
                .disabled(model?.appliedState.search == nil || model?.current.rowCount == 0)
            Button("Find Previous") { model?.findNext(backwards: true) }.keyboardShortcut(
                "g", modifiers: [.command, .shift]
            )
            .disabled(model?.appliedState.search == nil || model?.current.rowCount == 0)
        }
        SidebarCommands()
        CommandGroup(after: .sidebar) {
            if let workbook, let model {
                ForEach(InspectorPage.allCases, id: \.self) { page in
                    Toggle(
                        page.rawValue,
                        isOn: Binding(
                            get: { workbook.showsInspector && model.inspectorPage == page },
                            set: { selected in
                                guard selected else { return }
                                model.inspectorPage = page
                                workbook.showsInspector = true
                            })
                    )
                }
            }
            Divider()
            Button("Analyze Selected Column") {
                model?.inspectedColumn = model?.selectedColumn
                model?.inspectorPage = .column
                workbook?.showsInspector = true
            }
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
                .disabled(model?.canClearFilters != true)
        }
    }
}

private struct ExportCommand: View {
    @ObservedObject var model: DocumentModel
    @ObservedObject var export: ExportModel

    var body: some View {
        Button("Export…") { model.chooseExport() }
            .keyboardShortcut("e", modifiers: [.command, .shift])
            .disabled(model.isBusy || !export.isIdle)
    }
}
