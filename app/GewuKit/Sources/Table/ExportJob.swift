import Foundation
internal import GewuBindings

public enum ParquetCompression: String, CaseIterable, Sendable, Codable {
    case zstd, snappy, gzip, brotli, lz4, uncompressed

    var ffi: GewuBindings.ParquetCompression {
        switch self {
        case .zstd: .zstd
        case .snappy: .snappy
        case .gzip: .gzip
        case .brotli: .brotli
        case .lz4: .lz4
        case .uncompressed: .uncompressed
        }
    }
}

public struct ExportProgress: Hashable, Sendable {
    /// Rows processed. Reaching `totalRows` does not mean the file is in
    /// place; ``ExportJob/write(to:compression:progressInterval:onProgress:)`` returning does.
    public let rowsWritten: UInt64
    public let totalRows: UInt64

    public init(rowsWritten: UInt64, totalRows: UInt64) {
        self.rowsWritten = rowsWritten
        self.totalRows = totalRows
    }

    init(_ progress: GewuBindings.ExportProgress) {
        self.init(rowsWritten: progress.rowsWritten, totalRows: progress.totalRows)
    }
}

/// One export of captured rows and columns to one file. Single-use.
///
/// Writing goes to a sibling temporary file that replaces the destination
/// only on success, so a failed or cancelled export leaves it untouched.
public final class ExportJob: Sendable {
    public let format: FileType
    private let job: GewuBindings.GewuExport

    init(job: GewuBindings.GewuExport, format: FileType) {
        self.job = job
        self.format = format
    }

    /// Write the file on a worker of its own, reporting progress every
    /// `progressInterval` while it runs and once more when it succeeds; no
    /// report follows the return. Nested columns go to CSV, TSV and XLSX as
    /// their display text. Cancelling the calling task stops the write; it
    /// then throws `CancellationError` unless the file was already committed.
    public func write(
        to destination: URL, compression: ParquetCompression = .zstd,
        progressInterval: Duration = .milliseconds(150),
        onProgress: @escaping @MainActor @Sendable (ExportProgress) -> Void = { _ in }
    ) async throws {
        let job = job
        let path = destination.path
        let options = GewuBindings.ExportOptions(
            parquetCompression: compression.ffi, nestedAsText: true, includeHeader: true)
        let sampler = Task {
            while !Task.isCancelled {
                await onProgress(ExportProgress(job.progress()))
                try? await Task.sleep(for: progressInterval)
            }
        }
        do {
            try await withTaskCancellationHandler {
                try await SerialWorker.perform(alone: "export") {
                    try job.write(path: path, options: options)
                }
            } onCancel: {
                job.cancel()
            }
        } catch {
            sampler.cancel()
            await sampler.value
            throw error
        }
        sampler.cancel()
        await sampler.value
        await onProgress(ExportProgress(job.progress()))
    }
}
