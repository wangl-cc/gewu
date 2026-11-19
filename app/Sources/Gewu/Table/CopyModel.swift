import AppKit
import Observation
import UniformTypeIdentifiers

/// Copying is a TSV export of the selected display rows into memory. The export
/// captures an immutable View, so later queries do not change what is copied.
/// A copy is fast or exceeds `byteLimit`; it shows no progress, and a newer
/// copy cancels an unfinished one.
@MainActor
@Observable
final class CopyModel {
    /// Larger selections belong in an exported file, not on the pasteboard.
    static let byteLimit: UInt64 = 64 * 1024 * 1024

    private(set) var message: String?
    @ObservationIgnored private var job: GewuExport?
    @ObservationIgnored private var request: UInt64 = 0
    @ObservationIgnored private let pasteboard: NSPasteboard
    @ObservationIgnored private let reportError: (Error, String) -> Void

    init(pasteboard: NSPasteboard = .general, reportError: @escaping (Error, String) -> Void) {
        self.pasteboard = pasteboard
        self.reportError = reportError
    }

    func copy(ordered: GewuOrderedView, rows: [Range<Int>], columns: [ColumnId]) {
        cancel()
        message = nil
        let job: GewuExport
        do {
            job = try ordered.exportRows(
                ranges: rows.map { RowSpan(start: UInt64($0.lowerBound), count: UInt32(clamping: $0.count)) },
                columns: columns, format: .tsv)
        } catch {
            reportError(error, "Could not copy: ")
            return
        }
        self.job = job
        let mine = request
        let header = UserDefaults.standard.object(forKey: "CopyIncludesHeaders") as? Bool ?? true
        let limit = Self.byteLimit
        Task { [weak self] in
            let result = await Task.detached(priority: .userInitiated) {
                Result { try job.writeText(header: header, byteLimit: limit) }
            }.value
            guard let self, self.request == mine else { return }
            self.job = nil
            switch result {
            case .success(let text):
                let item = NSPasteboardItem()
                item.setData(text, forType: .string)
                item.setData(text, forType: NSPasteboard.PasteboardType(UTType.tabSeparatedText.identifier))
                self.pasteboard.clearContents()
                self.pasteboard.writeObjects([item])
                self.message = "Copied selection"
            case .failure(GewuError.Cancelled):
                break
            case .failure(GewuError.Limit(kind: .textBytes, _)):
                self.message = "Selection is too large to copy; use File > Export…"
            case .failure(let error):
                self.reportError(error, "Could not copy: ")
            }
        }
    }

    func cancel() {
        job?.cancel()
        job = nil
        request &+= 1
    }
}
