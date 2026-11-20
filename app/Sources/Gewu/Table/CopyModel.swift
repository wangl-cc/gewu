import AppKit
import GewuKit
import Observation
import UniformTypeIdentifiers

/// Copies the display text of selected cells as plain text and TSV.
@MainActor
@Observable
final class CopyModel {
    enum Outcome: Equatable {
        case copied
        case tooLarge
    }

    static let byteLimit = 64 * 1024 * 1024

    private(set) var outcome: Outcome?
    @ObservationIgnored private var work: Task<Void, Never>?
    @ObservationIgnored private let pasteboard: NSPasteboard
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let alerts: Alerts

    init(alerts: Alerts, pasteboard: NSPasteboard = .general, defaults: UserDefaults = .standard) {
        self.alerts = alerts
        self.pasteboard = pasteboard
        self.defaults = defaults
    }

    /// Copy `rows` of `columns` in display order. A newer copy replaces an unfinished one.
    func copy(from displayed: DisplayRows, rows: [Range<Int>], columns: [ColumnID]) {
        cancel()
        outcome = nil
        let includeHeader = defaults[Preferences.copyIncludesHeaders]
        work = Task {
            do {
                let text = try await displayed.tabSeparatedText(
                    rows: rows, columns: columns, includeHeader: includeHeader, byteLimit: Self.byteLimit)
                try Task.checkCancellation()
                let item = NSPasteboardItem()
                item.setData(text, forType: .string)
                item.setData(text, forType: NSPasteboard.PasteboardType(UTType.tabSeparatedText.identifier))
                pasteboard.clearContents()
                pasteboard.writeObjects([item])
                outcome = .copied
            } catch is CancellationError {
            } catch is TextLimitExceeded {
                outcome = .tooLarge
            } catch {
                alerts.report(error, title: "Could Not Copy")
            }
            if !Task.isCancelled { work = nil }
        }
    }

    func cancel() {
        work?.cancel()
        work = nil
    }
}
