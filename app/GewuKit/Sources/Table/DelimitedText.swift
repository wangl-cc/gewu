import Foundation
internal import GewuBindings
internal import gewuFFI

extension DisplayRows {
    /// Display text of `rows` and `columns` as tab-separated values, the same
    /// encoding as a TSV export, with nested values as their display text.
    /// Fails with ``GewuKitError/ExportProblem/sizeLimit(_:)`` instead of
    /// building more than `byteLimit` bytes. Runs on its own thread; cancelling the calling task stops it.
    public func tabSeparatedText(
        rows: [Range<Int>], columns: [ColumnID], includeHeader: Bool, byteLimit: Int
    ) async throws -> Data {
        let ranges = rows.map {
            GewuBindings.RowRange(start: UInt64(max($0.lowerBound, 0)), end: UInt64(max($0.upperBound, 0)))
        }
        let job = try translatingErrors {
            try view.exportRows(ranges: ranges, columns: columns.map(\.ffi), format: .tsv)
        }
        let options = GewuBindings.ExportOptions(
            parquetCompression: .zstd, nestedAsText: true, includeHeader: includeHeader)
        return try await withTaskCancellationHandler {
            try await SerialWorker(label: "copy").perform { () throws -> Data in
                let exported = try job.writeToMemory(options: options, limit: UInt64(byteLimit))
                let contents = gewu_exported_bytes(exported.uniffiCloneHandle())
                guard let bytes = contents.bytes else { return Data() }
                // One copy out of Rust, kept alive by `exported` until it completes.
                return withExtendedLifetime(exported) { Data(bytes: bytes, count: contents.count) }
            }
        } onCancel: {
            job.cancel()
        }
    }
}
