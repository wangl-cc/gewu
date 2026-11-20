import Foundation
import GewuKit

extension Filter {
    /// A compact condition description for query summaries and the header tooltip.
    /// This is display text only; filtering continues to use the original literals.
    var conditionSummary: String {
        let description: String
        switch condition {
        case .exact(let values, let exclude):
            if values.isEmpty {
                if exclude { return includeNulls ? "All values" : "Not missing" }
                return includeNulls ? "Only missing" : "No values"
            }
            description =
                values.count == 1
                ? "\(exclude ? "≠" : "=") \(Self.quoted(values[0]))"
                : "\(exclude ? "Except " : "")\(values.count) values"
        case .range(let min, let max):
            switch (min, max) {
            case (.some(let lo), .some(let hi)): description = "\(lo) … \(hi)"
            case (.some(let lo), nil): description = "≥ \(lo)"
            case (nil, .some(let hi)): description = "≤ \(hi)"
            case (nil, nil): return includeNulls ? "All values" : "Not missing"
            }
        case .text(let text, let mode, _):
            description = "\(mode == .equals ? "=" : "Contains") \(Self.quoted(text))"
        }
        return description + (includeNulls ? " · + missing" : "")
    }

    var detailSummary: String {
        var text = conditionSummary
        if case .exact(let values, _) = condition, values.count > 1 {
            text += "\n" + values.prefix(8).map(Self.quoted).joined(separator: ", ")
            if values.count > 8 { text += " …" }
        }
        if case .text(_, _, let caseSensitive) = condition {
            text += caseSensitive ? "\nCase sensitive" : "\nCase insensitive"
        }
        text += includeNulls ? "\nIncludes missing values" : "\nExcludes missing values"
        return text
    }

    private static func quoted(_ value: String) -> String {
        if value.isEmpty { return "(empty string)" }
        // Keep tooltips bounded even for very large text literals.
        let units = Array(value.utf16.prefix(161))
        let prefix = String(decoding: units.prefix(160), as: UTF16.self)
        let text = prefix.replacingOccurrences(of: "\n", with: "\\n")
            .replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\t", with: "\\t")
        return "“\(text)\(units.count > 160 ? "…" : "")”"
    }
}
