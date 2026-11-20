import AppKit
import GewuKit

/// A delayed sample may only replace the width for which it was requested.
/// Manual changes and subsequent Fit commands invalidate older tickets.
struct ColumnWidths {
    struct Ticket: Equatable {
        let column: ColumnID
        let revision: UInt64
    }
    private struct Entry {
        var revision: UInt64 = 0
        var settled = false
        var manual = false
    }
    private var entries: [ColumnID: Entry] = [:]

    func needsAutomatic(_ column: ColumnID) -> Bool { !(entries[column]?.settled ?? false) }

    mutating func begin(_ column: ColumnID) -> Ticket {
        var entry = entries[column] ?? Entry()
        entry.revision &+= 1
        entry.settled = true
        entries[column] = entry
        return Ticket(column: column, revision: entry.revision)
    }

    mutating func manual(_ column: ColumnID) {
        _ = begin(column)
        entries[column]?.manual = true
    }

    mutating func accept(_ ticket: Ticket) -> Bool {
        guard entries[ticket.column]?.revision == ticket.revision else { return false }
        entries[ticket.column]?.manual = false
        return true
    }

    mutating func abandon(_ ticket: Ticket) {
        guard entries[ticket.column]?.revision == ticket.revision else { return }
        let manual = entries[ticket.column]?.manual ?? false
        entries[ticket.column]?.settled = manual
    }

    mutating func fontChanged() {
        for column in Array(entries.keys) {
            guard var entry = entries[column] else { continue }
            entry.revision &+= 1
            entry.settled = entry.manual
            entries[column] = entry
        }
    }
}

enum ColumnWidthMeasurement {
    struct Input: Sendable {
        let column: ColumnID
        let name: String
        let type: String
        let text: Bool
        /// Already loaded text covering the sampled rows, if any.
        var sample: ColumnText?
    }

    /// Rows sampled per column.
    static let sampleRows = 32

    /// Fitted widths for `columns`, sampling up to ``sampleRows`` rows from
    /// `start`. Reads and measures off the main actor, checking cancellation
    /// between columns. UTF-16 input is limited before measuring so a single
    /// enormous composed character cannot make sampling unbounded.
    @concurrent
    nonisolated static func measure(
        rows: DisplayRows, columns: [Input], start: Int, fontSize: CGFloat, maximum: CGFloat
    ) async throws -> [ColumnID: CGFloat] {
        let sampled = max(0, start)..<min(rows.count, max(0, start) + sampleRows)
        func width(_ text: String, font: NSFont) -> CGFloat {
            let sample = String(decoding: text.utf16.prefix(512), as: UTF16.self)
            return ceil((sample as NSString).size(withAttributes: [.font: font]).width)
        }
        var result: [ColumnID: CGFloat] = [:]
        for column in columns {
            try Task.checkCancellation()
            let text: ColumnText
            if let sample = column.sample, sample.rows.contains(sampled.lowerBound),
                sample.rows.upperBound >= sampled.upperBound
            {
                text = sample
            } else {
                text = try await rows.text(of: column.column, rows: sampled)
            }
            var size =
                max(
                    width(column.name, font: .systemFont(ofSize: fontSize, weight: .semibold)),
                    width(column.type, font: .systemFont(ofSize: fontSize - 2))) + 60
            let font =
                column.text
                ? NSFont.systemFont(ofSize: fontSize) : .monospacedDigitSystemFont(ofSize: fontSize, weight: .regular)
            for row in sampled {
                size = max(size, width(text.cell(row) ?? "null", font: font) + 20)
            }
            result[column.column] = min(max(size, 80), maximum)
        }
        try Task.checkCancellation()
        return result
    }
}
