import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Bridges file-open events and native panels; SwiftUI owns the windows and menus.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        var args = CommandLine.arguments.dropFirst().makeIterator()
        while let argument = args.next() {
            if argument == "--" { continue }
            if argument.hasPrefix("-") { _ = args.next(); continue }
            FileCoordinator.shared.open(URL(fileURLWithPath: argument))
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        urls.forEach { FileCoordinator.shared.open($0) }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows { FileCoordinator.shared.showWelcome?() }
        return true
    }
}

@MainActor
final class FileCoordinator: ObservableObject {
    static let shared = FileCoordinator()
    @Published private(set) var recentURLs = NSDocumentController.shared.recentDocumentURLs
    var showWelcome: (() -> Void)?
    private var openWindow: ((URL) -> Void)?
    private var pending: [URL] = []

    func connect(openWindow: @escaping (URL) -> Void, showWelcome: @escaping () -> Void) {
        self.openWindow = openWindow
        self.showWelcome = showWelcome
        let queued = pending
        pending.removeAll()
        queued.forEach(open)
    }

    func open(_ url: URL) {
        guard url.isFileURL else { return }
        let canonical = url.standardizedFileURL.resolvingSymlinksInPath()
        if let openWindow { openWindow(canonical) }
        else { pending.append(canonical) }
    }

    func chooseFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = ExportFileFormat.allCases.map(\.contentType)
        if panel.runModal() == .OK { panel.urls.forEach(open) }
    }

    func recordRecent(_ url: URL) {
        NSDocumentController.shared.noteNewRecentDocumentURL(url)
        recentURLs = NSDocumentController.shared.recentDocumentURLs
    }

    func clearRecent() {
        NSDocumentController.shared.clearRecentDocuments(nil)
        recentURLs = NSDocumentController.shared.recentDocumentURLs
    }
}
