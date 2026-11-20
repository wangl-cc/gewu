import AppKit
import Observation
import UniformTypeIdentifiers

/// Copies the displayed text of selected cells as TSV.
@MainActor
@Observable
final class CopyModel {
    static let byteLimit = 64 * 1024 * 1024

    private(set) var message: String?
    @ObservationIgnored private var work: Task<Data, Error>?
    @ObservationIgnored private var request: UInt64 = 0
    @ObservationIgnored private let pasteboard: NSPasteboard
    @ObservationIgnored private let reportError: (Error, String) -> Void

    init(pasteboard: NSPasteboard = .general, reportError: @escaping (Error, String) -> Void) {
        self.pasteboard = pasteboard
        self.reportError = reportError
    }

    func copy(ordered: GewuOrderedView, rows: [Range<Int>], columns: [ColumnId], names: [String]) {
        cancel()
        message = nil
        let mine = request
        let header = UserDefaults.standard.object(forKey: "CopyIncludesHeaders") as? Bool ?? true
        let limit = Self.byteLimit
        let work = Task.detached(priority: .userInitiated) {
            try CopyText.format(
                ordered: ordered, rows: rows, columns: columns, names: names, header: header, limit: limit)
        }
        self.work = work
        Task { [weak self] in
            let result = await work.result
            guard let self, self.request == mine else { return }
            self.work = nil
            switch result {
            case .success(let text):
                let item = NSPasteboardItem()
                item.setData(text, forType: .string)
                item.setData(text, forType: NSPasteboard.PasteboardType(UTType.tabSeparatedText.identifier))
                self.pasteboard.clearContents()
                self.pasteboard.writeObjects([item])
                self.message = "Copied selection"
            case .failure(is CancellationError):
                break
            case .failure(is CopyText.TooLarge):
                self.message = "Selection is too large to copy; use File > Export…"
            case .failure(let error):
                self.reportError(error, "Could not copy: ")
            }
        }
    }

    func cancel() {
        work?.cancel()
        work = nil
        request &+= 1
    }
}

enum CopyText {
    struct TooLarge: Error {}

    /// Rows formatted per column read. Large reads keep FFI calls few; the
    /// byte limit still bounds the result.
    static let rowsPerRead = 4096

    static func format(
        ordered: GewuOrderedView, rows: [Range<Int>], columns: [ColumnId], names: [String], header: Bool,
        limit: Int
    ) throws -> Data {
        try Task.checkCancellation()
        var data = Data()
        var hasLine = false
        func append(_ line: String) throws {
            let separator = hasLine ? 1 : 0
            guard line.utf8.count <= limit - data.count - separator else { throw TooLarge() }
            if hasLine { data.append(10) }
            data.append(contentsOf: line.utf8)
            hasLine = true
        }
        if header { try append(names.map(escaped).joined(separator: "\t")) }
        let rowCount = Int(clamping: ordered.rowCount())
        for range in rows {
            let end = min(range.upperBound, rowCount)
            for start in stride(from: min(range.lowerBound, end), to: end, by: rowsPerRead) {
                try Task.checkCancellation()
                let count = min(rowsPerRead, end - start)
                var lines = Array(repeating: "", count: count)
                let separators = hasLine ? count : count - 1
                var remaining = limit - data.count - separators
                for (index, column) in columns.enumerated() {
                    try Task.checkCancellation()
                    let segment = try ordered.formatColumn(column: column, start: UInt64(start), count: UInt32(count))
                    for row in 0..<count {
                        let value = escaped(segment.cell(row: row) ?? "")
                        remaining -= value.utf8.count + (index > 0 ? 1 : 0)
                        guard remaining >= 0 else { throw TooLarge() }
                        if index > 0 { lines[row].append("\t") }
                        lines[row].append(value)
                    }
                }
                for line in lines { try append(line) }
            }
        }
        return data
    }

    static func escaped(_ value: String) -> String {
        guard value.utf8.contains(where: { $0 == 9 || $0 == 10 || $0 == 13 || $0 == 34 }) else { return value }
        return "\"" + value.replacingOccurrences(of: "\"", with: "\"\"") + "\""
    }
}
