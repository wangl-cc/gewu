import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

/// Bridges file-open events and native panels; SwiftUI owns the windows and menus.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        NotificationCenter.default.addObserver(
            self, selector: #selector(didFinishRestoringWindows),
            name: NSApplication.didFinishRestoringWindowsNotification, object: nil)
    }

    @objc private func didFinishRestoringWindows(_ notification: Notification) {
        FileCoordinator.shared.didFinishRestoringWindows()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        var args = CommandLine.arguments.dropFirst().makeIterator()
        while let argument = args.next() {
            if argument == "--" { continue }
            if argument.hasPrefix("-") {
                _ = args.next()
                continue
            }
            FileCoordinator.shared.open(URL(fileURLWithPath: argument))
        }
        FileCoordinator.shared.didFinishLaunching()
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls { FileCoordinator.shared.open(url) }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        // Let AppKit bring back minimized viewer windows instead of presenting a picker.
        if FileCoordinator.hasViewerWindows(in: sender.windows) { return true }
        FileCoordinator.shared.chooseFiles()
        return false
    }
}

@MainActor
final class FileCoordinator: ObservableObject {
    static let shared = FileCoordinator()
    @Published private(set) var recentURLs = NSDocumentController.shared.recentDocumentURLs
    private var openWindow: ((URL) -> Void)?
    private var pending: [URL] = []
    private var panel: NSOpenPanel?
    private var launchFinished = false
    private var restorationFinished = false
    private var initialPresentationHandled = false
    private var receivedDocument = false
    private let makePanel: @MainActor () -> NSOpenPanel
    private let hasExistingWindows: @MainActor () -> Bool

    static func hasViewerWindows(in windows: [NSWindow]) -> Bool {
        windows.contains { window in
            (window.isVisible || window.isMiniaturized)
                && window.representedURL != nil
        }
    }

    init(
        makePanel: @escaping @MainActor () -> NSOpenPanel = { NSOpenPanel() },
        hasExistingWindows: @escaping @MainActor () -> Bool = {
            FileCoordinator.hasViewerWindows(in: NSApp.windows)
        }
    ) {
        self.makePanel = makePanel
        self.hasExistingWindows = hasExistingWindows
    }

    func connect(openWindow: @escaping (URL) -> Void) {
        self.openWindow = openWindow
        let queued = pending
        pending.removeAll()
        queued.forEach(open)
    }

    func open(_ url: URL) {
        guard url.isFileURL else { return }
        receivedDocument = true
        let previousPanel = panel
        panel = nil
        previousPanel?.cancel(nil)
        let canonical = url.standardizedFileURL.resolvingSymlinksInPath()
        if let openWindow { openWindow(canonical) } else { pending.append(canonical) }
    }

    func documentPresented() { receivedDocument = true }

    func didFinishLaunching() {
        launchFinished = true
        presentInitialPanelIfNeeded()
    }

    func didFinishRestoringWindows() {
        restorationFinished = true
        presentInitialPanelIfNeeded()
    }

    private func presentInitialPanelIfNeeded() {
        guard launchFinished, restorationFinished, !initialPresentationHandled else { return }
        initialPresentationHandled = true
        guard !receivedDocument, !hasExistingWindows() else { return }
        chooseFiles()
    }

    func chooseFiles() {
        // An explicit Open request also consumes the initial launch invitation.
        initialPresentationHandled = true
        if let panel {
            panel.makeKeyAndOrderFront(nil)
            return
        }
        let panel = makePanel()
        self.panel = panel
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = ExportFileFormat.allCases.map(\.contentType)
        panel.begin { [weak self, weak panel] response in
            guard let self, let panel, self.panel === panel else { return }
            self.panel = nil
            if response == .OK { panel.urls.forEach(self.open) }
        }
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
