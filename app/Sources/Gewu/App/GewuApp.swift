import SwiftUI

@main
struct GewuApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var files = FileCoordinator.shared
    @StateObject private var updater = AppUpdater()
    @Environment(\.openWindow) private var openWindow
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup("Gewu", id: "viewer", for: URL.self) { $url in
            ViewerScene(url: url).environmentObject(files)
        }
        .defaultSize(width: 1200, height: 720)
        .windowToolbarStyle(.unified)
        .defaultLaunchBehavior(.suppressed)
        .commands {
            ViewerCommands(files: files)
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") { updater.checkForUpdates() }
                    .disabled(!updater.canCheckForUpdates)
            }
        }
        .onChange(of: scenePhase, initial: true) {
            // The app must be able to open its first file without an existing view.
            files.connect { openWindow(id: "viewer", value: $0) }
        }

        WindowGroup("File Info", id: "file-info", for: FileDetails.self) { $details in
            if let details { FileInfoView(details: details) }
        }
        .defaultSize(width: 460, height: 440)
        .windowResizability(.contentMinSize)
        .auxiliaryScene()

        Settings { SettingsView(updater: updater) }
            .windowResizability(.contentSize)
            .auxiliaryScene()
    }
}

extension Scene {
    /// Auxiliary scenes must never auto-open at launch or restore across restarts.
    fileprivate func auxiliaryScene() -> some Scene {
        defaultLaunchBehavior(.suppressed)
            .restorationBehavior(.disabled)
    }
}

private struct ViewerScene: View {
    let url: URL?
    @EnvironmentObject private var files: FileCoordinator
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Group {
            if let url {
                DocumentScene(url: url).id(url)
                    .onAppear { files.documentPresented() }
            } else {
                // Discard an obsolete empty window restored from an older version.
                Color.clear.task { dismiss() }
            }
        }
        .onOpenURL { files.open($0) }
    }
}
