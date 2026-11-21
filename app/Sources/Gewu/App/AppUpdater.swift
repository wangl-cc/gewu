import Foundation
import Observation
import Sparkle

/// Bridges Sparkle's preferences and availability to SwiftUI. Sparkle owns
/// scheduling, persistence, signature verification and installation.
@MainActor
@Observable
final class AppUpdater {
    private(set) var canCheckForUpdates = false
    var automaticallyChecksForUpdates = false {
        didSet {
            if updater?.automaticallyChecksForUpdates != automaticallyChecksForUpdates {
                updater?.automaticallyChecksForUpdates = automaticallyChecksForUpdates
            }
        }
    }
    var automaticallyDownloadsUpdates = false {
        didSet {
            if updater?.automaticallyDownloadsUpdates != automaticallyDownloadsUpdates {
                updater?.automaticallyDownloadsUpdates = automaticallyDownloadsUpdates
            }
        }
    }

    @ObservationIgnored private let controller: SPUStandardUpdaterController?
    @ObservationIgnored private var observations: [NSKeyValueObservation] = []

    /// `nil` for local builds without the release key, which must not offer production updates.
    var updater: SPUUpdater? { controller?.updater }

    init() {
        guard let key = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String, !key.isEmpty else {
            controller = nil
            return
        }
        let controller = SPUStandardUpdaterController(
            startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
        self.controller = controller
        let updater = controller.updater
        // Sparkle changes these on the main thread; mirror them for SwiftUI.
        observations = [
            updater.observe(\.canCheckForUpdates, options: [.initial, .new]) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.canCheckForUpdates = updater.canCheckForUpdates }
            },
            updater.observe(\.automaticallyChecksForUpdates, options: [.initial, .new]) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.automaticallyChecksForUpdates = updater.automaticallyChecksForUpdates }
            },
            updater.observe(\.automaticallyDownloadsUpdates, options: [.initial, .new]) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.automaticallyDownloadsUpdates = updater.automaticallyDownloadsUpdates }
            },
        ]
        controller.startUpdater()
    }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}
