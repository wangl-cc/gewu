import GewuKit
import SwiftUI

struct ViewerCommands: Commands {
    let files: FileCoordinator
    @FocusedValue(WindowModel.self) private var window
    @FocusedValue(TableModel.self) private var model
    @AppStorage(Preferences.showsRowNumbers) private var showsRowNumbers: Bool
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
            Button("Import Options…") { window?.showsImportOptions = true }
                .disabled(window?.supportsHeaderOption != true || window?.isOpening == true)
            Button("Get Info…") {
                if let info = model?.info { openWindow(id: "file-info", value: FileDetails(info: info)) }
            }.keyboardShortcut("i").disabled(model == nil)
        }
        CommandGroup(after: .newItem) {
            Divider()
            Button("Export…") {
                guard let model, model.canExport, let window = NSApp.keyWindow else { return }
                model.export.choose(
                    rows: model.displayed, columns: model.visibleColumns, source: model.url, window: window)
            }
            .keyboardShortcut("e", modifiers: [.command, .shift])
            .disabled(model?.canExport != true)
        }
        CommandGroup(after: .textEditing) {
            Divider()
            Button("Find…") { window?.isSearchFocused = true }.keyboardShortcut("f").disabled(model == nil)
            Button("Find Next") { model?.find.next() }.keyboardShortcut("g")
                .disabled(model?.find.canNavigate != true)
            Button("Find Previous") { model?.find.next(backwards: true) }
                .keyboardShortcut("g", modifiers: [.command, .shift])
                .disabled(model?.find.canNavigate != true)
        }
        InspectorCommands()
        CommandGroup(after: .sidebar) {
            Button("Columns…") { model?.showsColumns = true }.disabled(model == nil)
            Button("Sort…") { model?.showsSorting = true }.disabled(model == nil)
            Divider()
            Button("Show Chart Inspector") {
                window?.inspectorPage = .chart
                window?.showsInspector = true
            }.disabled(model == nil)
            Divider()
            Button("Analyze Selected Column") {
                if let column = model?.selectedColumn { window?.inspect(column) }
            }
            .keyboardShortcut("i", modifiers: [.command, .option])
            .disabled(model?.selectedColumn == nil || model?.isUpdating == true)
            Toggle("Show Row Numbers", isOn: $showsRowNumbers).keyboardShortcut("r", modifiers: [.command, .option])
                .disabled(model == nil)
        }
        CommandMenu("Filter") {
            Button("Filter Column…") {
                if let column = model?.selectedColumn { model?.editFilter(column) }
            }
            .keyboardShortcut("f", modifiers: [.command, .option])
            .disabled(model?.selectedColumn == nil)
            Divider()
            Button("Clear All Filters") { model?.clearFilters() }.keyboardShortcut("l", modifiers: [.command, .shift])
                .disabled(model?.canClearFilters != true)
        }
    }
}
