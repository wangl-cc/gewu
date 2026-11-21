import AppKit
import GewuKit
import SwiftUI
import UniformTypeIdentifiers

/// Bridges file-open events and native panels; SwiftUI owns the windows and menus.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var restorationObserver: (any NSObjectProtocol)?

    func applicationWillFinishLaunching(_ notification: Notification) {
        restorationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didFinishRestoringWindowsNotification, object: NSApp, queue: .main
        ) { _ in
            MainActor.assumeIsolated { FileCoordinator.shared.didFinishRestoringWindows() }
        }
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
@Observable
final class FileCoordinator {
    static let shared = FileCoordinator()
    private(set) var recentURLs = NSDocumentController.shared.recentDocumentURLs
    @ObservationIgnored private var openWindow: ((URL) -> Void)?
    @ObservationIgnored private var pending: [URL] = []
    @ObservationIgnored private var panel: NSOpenPanel?
    @ObservationIgnored private var launchFinished = false
    @ObservationIgnored private var restorationFinished = false
    @ObservationIgnored private var initialPresentationHandled = false
    @ObservationIgnored private var receivedDocument = false
    @ObservationIgnored private let makePanel: @MainActor () -> NSOpenPanel
    @ObservationIgnored private let hasExistingWindows: @MainActor () -> Bool

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
