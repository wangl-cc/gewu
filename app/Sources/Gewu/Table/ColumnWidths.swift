import AppKit

/// A delayed sample may only replace the width for which it was requested.
/// Manual changes and subsequent Fit commands invalidate older tickets.
struct ColumnWidths {
    struct Ticket: Equatable {
        let column: UInt32
        let revision: UInt64
    }
    private struct Entry {
        var revision: UInt64 = 0
        var settled = false
        var manual = false
    }
    private var entries: [UInt32: Entry] = [:]

    func needsAutomatic(_ column: UInt32) -> Bool { !(entries[column]?.settled ?? false) }

    mutating func begin(_ column: UInt32) -> Ticket {
        var entry = entries[column] ?? Entry()
        entry.revision &+= 1
        entry.settled = true
        entries[column] = entry
        return Ticket(column: column, revision: entry.revision)
    }

    mutating func manual(_ column: UInt32) {
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
        let column: UInt32
        let name: String
        let type: String
        let text: Bool
    }

    /// Runs on a serial worker. Limit UTF-16 input before grapheme processing so
    /// a single enormous composed character cannot make sampling unbounded.
    static func measure(
        view: GewuView, columns: [Input], start: UInt64, fontSize: CGFloat,
        maximum: CGFloat, cancellation: Request
    ) throws -> [UInt32: CGFloat] {
        var result: [UInt32: CGFloat] = [:]
        func width(_ text: String, font: NSFont) -> CGFloat {
            let sample = String(decoding: text.utf16.prefix(512), as: UTF16.self)
            return ceil((sample as NSString).size(withAttributes: [.font: font]).width)
        }
        for column in columns {
            try cancellation.check()
            let block = try view.rows(start: start, count: 32, columns: [column.column])
            var size =
                max(
                    width(column.name, font: .systemFont(ofSize: fontSize, weight: .semibold)),
                    width(column.type, font: .systemFont(ofSize: fontSize - 2))) + 60
            let font =
                column.text
                ? NSFont.systemFont(ofSize: fontSize) : .monospacedDigitSystemFont(ofSize: fontSize, weight: .regular)
            for cell in block.cells { size = max(size, width(cell ?? "null", font: font) + 20) }
            result[column.column] = min(max(size, 80), maximum)
        }
        try cancellation.check()
        return result
    }
}
