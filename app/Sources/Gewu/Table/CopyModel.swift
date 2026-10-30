import AppKit
import Combine

/// A copy captures row ranges, display order and a version, independent of the
/// viewport cache. Only the final pasteboard commit runs on the main actor.
@MainActor
final class CopyModel: ObservableObject {
    @Published private(set) var isCopying = false
    @Published private(set) var message: String?
    private var task: Task<String, Error>?
    private var request: UInt64 = 0
    private let reportError: (Error, String) -> Void

    init(reportError: @escaping (Error, String) -> Void) { self.reportError = reportError }

    func copy(document: GewuDocument, version: UInt64, rows: [Range<Int>], columns: [UInt32], names: [String]) {
        cancel()
        request &+= 1
        let mine = request
        isCopying = true
        message = nil
        let includeHeaders = UserDefaults.standard.object(forKey: "CopyIncludesHeaders") as? Bool ?? true
        let work = Task.detached(priority: .userInitiated) {
            try CopyText.format(rows: rows, columns: columns, names: names, includeHeaders: includeHeaders) {
                start, count, columns in
                try document.rows(version: version, start: UInt64(start), count: UInt32(count), columns: columns)
            }
        }
        task = work
        Task { [weak self] in
            do {
                let text = try await work.value
                guard let self, self.request == mine else { return }
                let current = document.currentView().version
                guard current == version else { throw GewuError.StaleVersion(requested: version, current: current) }
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                self.message = "Copied selection"
                self.isCopying = false
                self.task = nil
            } catch {
                guard let self, self.request == mine else { return }
                self.isCopying = false
                self.task = nil
                if error is CancellationError {
                    self.message = "Copy cancelled"
                } else {
                    self.reportError(error, "Could not copy: ")
                }
            }
        }
    }

    func cancel() {
        task?.cancel()
        task = nil
        request &+= 1
        if isCopying { message = "Copy cancelled" }
        isCopying = false
    }
}

/// Bounded FFI batches, with cancellation between batches and individual rows.
/// The final TSV necessarily occupies memory proportional to the selection.
enum CopyText {
    static func format(
        rows: [Range<Int>], columns: [UInt32], names: [String], includeHeaders: Bool = true,
        fetch: (Int, Int, [UInt32]) throws -> RowBlock
    ) throws -> String {
        var result = includeHeaders ? names.map(escaped).joined(separator: "\t") : ""
        var hasLine = includeHeaders
        for range in rows {
            var start = range.lowerBound
            while start < range.upperBound {
                try Task.checkCancellation()
                let count = min(64, range.upperBound - start)
                var lines = Array(repeating: "", count: count)
                for first in stride(from: 0, to: columns.count, by: 64) {
                    try Task.checkCancellation()
                    let selected = Array(columns[first..<min(first + 64, columns.count)])
                    let block = try fetch(start, count, selected)
                    guard Int(block.nRows) == count, Int(block.nCols) == selected.count else {
                        throw GewuError.Internal(message: "The selected rows could not be read completely.")
                    }
                    for row in 0..<count {
                        try Task.checkCancellation()
                        if first > 0 { lines[row] += "\t" }
                        let base = row * selected.count
                        lines[row] += block.cells[base..<(base + selected.count)].map { escaped($0 ?? "") }
                            .joined(separator: "\t")
                    }
                }
                for line in lines {
                    if hasLine { result += "\n" }
                    result += line
                    hasLine = true
                }
                start += count
            }
        }
        try Task.checkCancellation()
        return result
    }

    private static func escaped(_ value: String) -> String {
        value.contains(where: { "\t\n\r\"".contains($0) })
            ? "\"" + value.replacingOccurrences(of: "\"", with: "\"\"") + "\"" : value
    }
}
