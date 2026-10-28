import AppKit
import UniformTypeIdentifiers

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var windows: [DocumentWindowController] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = buildMenu()
        NSApp.activate(ignoringOtherApps: true)

        // `swift run Gewu -- path/to/file.parquet` opens the file directly.
        // Cocoa options come as "-Key value" pairs; skip both.
        var paths: [String] = []
        var args = CommandLine.arguments.dropFirst().makeIterator()
        while let arg = args.next() {
            if arg == "--" { continue }
            if arg.hasPrefix("-") { _ = args.next(); continue }
            paths.append(arg)
        }
        if paths.isEmpty {
            openDocument(nil)
        } else {
            paths.forEach { open(url: URL(fileURLWithPath: $0)) }
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        urls.forEach(open(url:))
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    @objc func openDocument(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        if let parquet = UTType(filenameExtension: "parquet") {
            panel.allowedContentTypes = [parquet]
        }
        guard panel.runModal() == .OK else { return }
        panel.urls.forEach(open(url:))
    }

    func open(url: URL) {
        let controller = DocumentWindowController(url: url)
        controller.onClose = { [weak self, weak controller] in
            self?.windows.removeAll { $0 === controller }
        }
        windows.append(controller)
        controller.showWindow(nil)
    }

    private func buildMenu() -> NSMenu {
        let main = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Quit Gewu", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let fileItem = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        fileMenu.addItem(withTitle: "Open…", action: #selector(openDocument(_:)), keyEquivalent: "o")
        fileMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        fileItem.submenu = fileMenu
        main.addItem(fileItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "Find", action: #selector(DocumentWindowController.focusSearch(_:)), keyEquivalent: "f")
        editMenu.addItem(withTitle: "Find Next", action: #selector(DocumentWindowController.findNext(_:)), keyEquivalent: "g")
        let previous = editMenu.addItem(
            withTitle: "Find Previous", action: #selector(DocumentWindowController.findPrevious(_:)), keyEquivalent: "g"
        )
        previous.keyEquivalentModifierMask = [.command, .shift]
        editItem.submenu = editMenu
        main.addItem(editItem)

        let viewItem = NSMenuItem()
        let viewMenu = NSMenu(title: "View")
        let inspector = viewMenu.addItem(
            withTitle: "Show Inspector", action: #selector(NSSplitViewController.toggleInspector(_:)), keyEquivalent: "i"
        )
        inspector.keyEquivalentModifierMask = [.command, .option]
        let rowNumbers = viewMenu.addItem(
            withTitle: "Show Row Numbers", action: #selector(DocumentWindowController.toggleRowNumbers(_:)),
            keyEquivalent: "r"
        )
        rowNumbers.keyEquivalentModifierMask = [.command, .option]
        viewItem.submenu = viewMenu
        main.addItem(viewItem)

        let filterItem = NSMenuItem()
        let filterMenu = NSMenu(title: "Filter")
        let clear = filterMenu.addItem(
            withTitle: "Clear All Filters", action: #selector(DocumentWindowController.clearAllFilters(_:)),
            keyEquivalent: "l"
        )
        clear.keyEquivalentModifierMask = [.command, .shift]
        filterItem.submenu = filterMenu
        main.addItem(filterItem)

        return main
    }
}
