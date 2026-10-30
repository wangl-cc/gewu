import AppKit

@MainActor
enum FileOpeningTests {
    static func run() {
        _ = NSApplication.shared
        for restorationFirst in [false, true] {
            let panel = TestOpenPanel()
            let files = FileCoordinator(makePanel: { panel }, hasExistingWindows: { false })
            if restorationFirst { files.didFinishRestoringWindows() } else { files.didFinishLaunching() }
            assert(panel.presentations == 0)
            if restorationFirst { files.didFinishLaunching() } else { files.didFinishRestoringWindows() }
            assert(panel.presentations == 1)
            assert(panel.accessoryView == nil, "Open uses the standard file picker without import controls")
            files.chooseFiles()
            assert(panel.presentations == 1 && panel.activations == 1)
            panel.cancel(nil)
            files.didFinishLaunching()
            files.didFinishRestoringWindows()
            assert(panel.presentations == 1, "Cancelling must not restart the launch picker")
        }

        var panels: [TestOpenPanel] = []
        let files = FileCoordinator(
            makePanel: {
                let panel = TestOpenPanel()
                panels.append(panel)
                return panel
            }, hasExistingWindows: { false })
        files.chooseFiles()
        panels[0].cancel(nil)
        files.didFinishLaunching()
        files.didFinishRestoringWindows()
        assert(panels.count == 1, "An early explicit Open consumes the launch invitation")

        files.chooseFiles()
        let staleCompletion = panels[1].completion
        let url = URL(fileURLWithPath: "/private/tmp/../tmp/gewu-ui-analysis.csv")
        files.open(url)
        assert(panels[1].cancellations == 1)
        var opened: [URL] = []
        files.connect { opened.append($0) }
        assert(opened == [url.standardizedFileURL.resolvingSymlinksInPath()])
        files.chooseFiles()
        staleCompletion?(.OK)
        files.chooseFiles()
        assert(panels.count == 3, "A stale completion must not discard the current panel")
        panels[2].cancel(nil)

        let restored = FileCoordinator(
            makePanel: { preconditionFailure("Restored windows must suppress the launch picker") },
            hasExistingWindows: { true })
        restored.didFinishRestoringWindows()
        restored.didFinishLaunching()

        let requested = FileCoordinator(
            makePanel: { preconditionFailure("A queued file must suppress the launch picker") },
            hasExistingWindows: { false })
        requested.open(url)
        requested.didFinishLaunching()
        requested.didFinishRestoringWindows()
        requested.connect { opened.append($0) }
        assert(opened.count == 2)
        requested.connect { _ in preconditionFailure("A consumed request must not be replayed") }

    }
}

/// Exercises the coordinator without displaying native windows or running a modal loop.
@MainActor
private final class TestOpenPanel: NSOpenPanel {
    var presentations = 0
    var activations = 0
    var cancellations = 0
    var completion: ((NSApplication.ModalResponse) -> Void)?

    override func begin(completionHandler handler: @escaping (NSApplication.ModalResponse) -> Void) {
        presentations += 1
        completion = handler
    }

    override func makeKeyAndOrderFront(_ sender: Any?) { activations += 1 }

    override func cancel(_ sender: Any?) {
        cancellations += 1
        completion?(.cancel)
    }
}
