import Foundation

/// Keep structured errors intact until they reach a user-facing message.
extension Error {
    var userMessage: String {
        guard let error = self as? GewuError else { return localizedDescription }
        switch error {
        case .Io(let message), .Unsupported(let message), .InvalidArgument(let message), .Internal(let message):
            return message
        case .Cancelled:
            return "The operation was cancelled."
        case .StaleVersion:
            return "The results changed. Please try again."
        }
    }
}
