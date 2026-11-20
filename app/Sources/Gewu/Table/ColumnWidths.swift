import AppKit

/// A delayed sample may only replace the width for which it was requested.
/// Manual changes and subsequent Fit commands invalidate older tickets.
struct ColumnWidths {
    struct Ticket: Equatable {
        let column: ColumnId
        let revision: UInt64
    }
    private struct Entry {
        var revision: UInt64 = 0
        var settled = false
        var manual = false
    }
    private var entries: [ColumnId: Entry] = [:]

    func needsAutomatic(_ column: ColumnId) -> Bool { !(entries[column]?.settled ?? false) }

    mutating func begin(_ column: ColumnId) -> Ticket {
        var entry = entries[column] ?? Entry()
        entry.revision &+= 1
        entry.settled = true
        entries[column] = entry
        return Ticket(column: column, revision: entry.revision)
    }

    mutating func manual(_ column: ColumnId) {
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
    // Row formatting has no Rust cancellation parameter. Stop between bounded
    // column samples, including before a queued request starts.
    final class Request: @unchecked Sendable {
        private let lock = NSLock()
        private var cancelled = false
        func cancel() {
            lock.lock()
            defer { lock.unlock() }
            cancelled = true
        }
        func check() throws {
            lock.lock()
            defer { lock.unlock() }
            if cancelled { throw CancellationError() }
        }
    }
    struct Input: Sendable {
        let column: ColumnId
        let name: String
        let type: String
        let text: Bool
        var sample: (segment: FormattedColumnSegment, row: Int)? = nil
    }

    /// Runs on a serial worker. Limit UTF-16 input before grapheme processing so
    /// a single enormous composed character cannot make sampling unbounded.
    static func measure(
        ordered: GewuOrderedView, columns: [Input], start: UInt64, fontSize: CGFloat,
        maximum: CGFloat, cancellation: Request
    ) throws -> [ColumnId: CGFloat] {
        var result: [ColumnId: CGFloat] = [:]
        let rowCount = ordered.rowCount()
        let count = Int(min(32, rowCount - min(start, rowCount)))
        func width(_ text: String, font: NSFont) -> CGFloat {
            let sample = String(decoding: text.utf16.prefix(512), as: UTF16.self)
            return ceil((sample as NSString).size(withAttributes: [.font: font]).width)
        }
        func measure(_ column: Input, cell: (Int) -> String?) {
            var size =
                max(
                    width(column.name, font: .systemFont(ofSize: fontSize, weight: .semibold)),
                    width(column.type, font: .systemFont(ofSize: fontSize - 2))) + 60
            let font =
                column.text
                ? NSFont.systemFont(ofSize: fontSize) : .monospacedDigitSystemFont(ofSize: fontSize, weight: .regular)
            for row in 0..<count {
                size = max(size, width(cell(row) ?? "null", font: font) + 20)
            }
            result[column.column] = min(max(size, 80), maximum)
        }
        for column in columns {
            try cancellation.check()
            if let sample = column.sample, sample.row + count <= Int(sample.segment.rowCount) {
                measure(column) { sample.segment.cell(row: sample.row + $0) }
            } else {
                let segment = try ordered.formatColumn(column: column.column, start: start, count: UInt32(count))
                measure(column) { segment.cell(row: $0) }
            }
        }
        try cancellation.check()
        return result
    }
}
