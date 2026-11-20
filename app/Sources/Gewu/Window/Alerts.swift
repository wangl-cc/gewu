import Foundation
import Observation

/// What went wrong, as the window shows it.
struct AlertMessage: Identifiable, Equatable {
    let id = UUID()
    let title: String
    let message: String
}

/// The window's single channel for failures. Models report here; the window
/// presents one alert at a time and keeps later reports until it is dismissed.
@MainActor
@Observable
final class Alerts {
    private(set) var queue: [AlertMessage] = []

    var current: AlertMessage? { queue.first }

    func report(_ error: any Error, title: String) {
        guard !(error is CancellationError) else { return }
        var message = error.localizedDescription
        if let reason = (error as? LocalizedError)?.failureReason { message += "\n\n" + reason }
        queue.append(AlertMessage(title: title, message: message))
    }

    func dismiss(_ alert: AlertMessage) {
        queue.removeAll { $0.id == alert.id }
    }
}
