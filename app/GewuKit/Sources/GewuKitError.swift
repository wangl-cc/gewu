import Foundation

/// A failure reported by the data engine, grouped by what the user can do about it.
///
/// Every case carries Core's message, which already names the operation,
/// value or limit involved. Cancellation is never a `GewuKitError`: GewuKit
/// reports it as `CancellationError`, whether Swift or Rust cancelled the work.
public enum GewuKitError: LocalizedError, Equatable, Sendable {
    /// The file could not be read, opened or written.
    case fileAccess(String)
    /// The file's content or format is not supported.
    case unreadableFile(String)
    /// The table exceeds a size limit.
    case tooLarge(String)
    /// A filter value the user typed is invalid.
    case invalidFilter(String)
    /// A search query could not be compiled.
    case invalidSearch(String)
    /// The selected rows do not support the requested analysis.
    case analysisUnavailable(String)
    /// An export could not be written.
    case exportFailed(String)
    /// The app called the engine incorrectly. Indicates a defect.
    case internalFailure(String)

    public var errorDescription: String? {
        switch self {
        case .fileAccess(let message), .unreadableFile(let message), .tooLarge(let message),
            .invalidFilter(let message), .invalidSearch(let message), .analysisUnavailable(let message),
            .exportFailed(let message), .internalFailure(let message):
            message
        }
    }
}

extension FfiError {
    var translated: any Error {
        switch self {
        case .Io(_, let message): GewuKitError.fileAccess(message)
        case .File(_, let message): GewuKitError.unreadableFile(message)
        case .Limit(_, let message): GewuKitError.tooLarge(message)
        case .Filter(_, let message): GewuKitError.invalidFilter(message)
        case .Find(_, let message): GewuKitError.invalidSearch(message)
        case .Analysis(_, let message): GewuKitError.analysisUnavailable(message)
        case .Export(_, let message): GewuKitError.exportFailed(message)
        case .Internal(let message): GewuKitError.internalFailure(message)
        case .Cancelled: CancellationError()
        }
    }
}
