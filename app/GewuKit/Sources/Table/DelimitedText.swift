import Foundation

/// The requested text would exceed its byte limit.
public struct TextLimitExceeded: Error, Sendable {
    public let limit: Int
}

extension DisplayRows {
    /// Rows formatted per Rust call. Large reads keep calls few; the byte
    /// limit still bounds the result.
    static let rowsPerTextRead = 4096

    /// Display text of `rows` and `columns` as tab-separated values, one line
    /// per row, with an optional header of column names. Fields holding a tab,
    /// line break or quote are quoted. Throws ``TextLimitExceeded`` instead of
    /// building more than `byteLimit` bytes. Runs on its own thread and stops
    /// between reads when the calling task is cancelled.
    public func tabSeparatedText(
        rows: [Range<Int>], columns: [ColumnID], includeHeader: Bool, byteLimit: Int
    ) async throws -> Data {
        let view = view
        let names = columns.map { source.column($0)?.name ?? "" }
        let rowCount = count
        return try await SerialWorker(label: "copy").perform {
            var output = TabSeparatedWriter(limit: byteLimit)
            if includeHeader {
                try output.appendLine(names.map { Array($0.utf8) })
            }
            for range in rows {
                let end = min(range.upperBound, rowCount)
                for start in stride(from: min(max(range.lowerBound, 0), end), to: end, by: Self.rowsPerTextRead) {
                    let count = min(Self.rowsPerTextRead, end - start)
                    var lines = Array(repeating: [[UInt8]](), count: count)
                    // Bytes the lines need so far, counting one line break and
                    // one tab per field: at most two more per line than written.
                    var pending = count
                    for column in columns {
                        try Task.checkCancellation()
                        let text = ColumnText(
                            try view.formatColumn(column: column.ffi, start: UInt64(start), count: UInt32(count)),
                            start: start)
                        for row in 0..<count {
                            let field = text.withUTF8(start + row) { Array($0) } ?? []
                            pending += TabSeparatedWriter.escapedCount(field) + 1
                            lines[row].append(field)
                        }
                        // Stop before gathering fields that cannot fit.
                        try output.check(atLeast: pending - 2 * count)
                    }
                    for line in lines { try output.appendLine(line) }
                }
            }
            return output.data
        }
    }
}

/// Accumulates TSV lines within a byte limit.
private struct TabSeparatedWriter {
    let limit: Int
    private(set) var data = Data()
    private var lines = 0

    init(limit: Int) { self.limit = limit }

    /// Fail when lines needing at least `bytes` more cannot fit.
    func check(atLeast bytes: Int) throws {
        if data.count + bytes > limit { throw TextLimitExceeded(limit: limit) }
    }

    mutating func appendLine(_ fields: [[UInt8]]) throws {
        var line: [UInt8] = []
        for (index, field) in fields.enumerated() {
            if index > 0 { line.append(9) }
            Self.appendEscaped(field, to: &line)
        }
        let separator = lines > 0 ? 1 : 0
        guard data.count + separator + line.count <= limit else { throw TextLimitExceeded(limit: limit) }
        if separator > 0 { data.append(10) }
        data.append(contentsOf: line)
        lines += 1
    }

    private static func needsQuotes(_ field: [UInt8]) -> Bool {
        field.contains { $0 == 9 || $0 == 10 || $0 == 13 || $0 == 34 }
    }

    static func escapedCount(_ field: [UInt8]) -> Int {
        needsQuotes(field) ? field.count + 2 + field.count(where: { $0 == 34 }) : field.count
    }

    private static func appendEscaped(_ field: [UInt8], to line: inout [UInt8]) {
        guard needsQuotes(field) else {
            line.append(contentsOf: field)
            return
        }
        line.append(34)
        for byte in field {
            line.append(byte)
            if byte == 34 { line.append(34) }
        }
        line.append(34)
    }
}
