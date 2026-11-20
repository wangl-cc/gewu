import Foundation
import Synchronization

public enum ParquetCompression: String, CaseIterable, Sendable, Codable {
    case zstd, snappy, gzip, brotli, lz4, uncompressed

    var ffi: FfiParquetCompression {
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
    /// place; ``ExportJob/write(to:compression:progress:)`` returning does.
    public let rowsWritten: UInt64
    public let totalRows: UInt64

    public init(rowsWritten: UInt64, totalRows: UInt64) {
        self.rowsWritten = rowsWritten
        self.totalRows = totalRows
    }
}

/// One export of captured rows and columns to one file. Single-use.
///
/// Writing goes to a sibling temporary file that replaces the destination
/// only on success, so a failed or cancelled export leaves it untouched.
public final class ExportJob: Sendable {
    public let format: FileType
    private let job: FfiExport
    private let finished = Mutex(false)

    init(job: FfiExport, format: FileType) {
        self.job = job
        self.format = format
    }

    /// Write the file on a dedicated thread. Cancelling the calling task stops
    /// the write; it then throws `CancellationError` unless the file was
    /// already committed.
    public func write(to destination: URL, compression: ParquetCompression = .zstd) async throws {
        let job = job
        let path = destination.path
        let options = FfiExportOptions(parquetCompression: compression.ffi)
        defer { finished.withLock { $0 = true } }
        try await withTaskCancellationHandler {
            try await SerialWorker(label: "export").perform {
                try job.writeWithOptions(path: path, options: options)
            }
        } onCancel: {
            job.cancel()
        }
    }

    /// Progress sampled every `interval` until ``write(to:compression:)`` returns.
    public func progress(every interval: Duration = .milliseconds(150)) -> AsyncStream<ExportProgress> {
        AsyncStream { continuation in
            let sampler = Task { [self] in
                while !Task.isCancelled {
                    let done = self.finished.withLock { $0 }
                    let current = self.job.progress()
                    continuation.yield(ExportProgress(rowsWritten: current.rowsWritten, totalRows: current.totalRows))
                    if done { break }
                    try? await Task.sleep(for: interval)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in sampler.cancel() }
        }
    }
}
