import Foundation

/// Compact presentation only. Never use these labels to reconstruct filter bounds.
/// Source text is retained separately for copying and tooltips.
enum ChartLabels {
    static func endpoints(_ lower: String, _ upper: String) -> (String, String) {
        if let a = timestamp(lower), let b = timestamp(upper) {
            let labels: (String, String)
            if a.date != b.date {
                if a.date.prefix(4) != b.date.prefix(4) {
                    labels = (String(a.date.prefix(7)), String(b.date.prefix(7)))
                } else {
                    labels = (String(a.date.suffix(5)), String(b.date.suffix(5)))
                }
            } else if !a.time.isEmpty && !b.time.isEmpty {
                labels = times(a.time, b.time)
            } else {
                labels = (a.date, b.date)
            }
            // DST can repeat a local clock time. Different offsets must remain
            // visible rather than turning a real interval into identical labels.
            if a.zone != b.zone { return (labels.0 + " " + a.zone, labels.1 + " " + b.zone) }
            return labels
        }
        if lower.contains(":"), upper.contains(":"), !lower.contains(" "), !upper.contains(" ") {
            return times(lower, upper)
        }
        return (number(lower), number(upper))
    }

    /// Shared date/zone context removed from the compact endpoint labels.
    static func context(_ lower: String, _ upper: String) -> String? {
        guard let a = timestamp(lower), let b = timestamp(upper) else { return nil }
        var parts: [String] = []
        if a.date == b.date, !a.time.isEmpty {
            parts.append(a.date)
        } else if a.date.prefix(4) == b.date.prefix(4) {
            parts.append(String(a.date.prefix(4)))
        }
        if a.zone == b.zone, !a.zone.isEmpty { parts.append(a.zone == "Z" ? "UTC" : a.zone) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    static func number(_ text: String) -> String {
        guard text.count > 12 else { return text }
        var mantissa = text
        let negative = mantissa.hasPrefix("-")
        if negative || mantissa.hasPrefix("+") { mantissa.removeFirst() }
        let exponentParts = mantissa.lowercased().split(separator: "e", omittingEmptySubsequences: false)
        guard exponentParts.count <= 2 else { return text }
        let explicitExponent = exponentParts.count == 2 ? Int(exponentParts[1]) : 0
        guard let explicitExponent else { return text }
        let components = exponentParts[0].split(separator: ".", omittingEmptySubsequences: false)
        guard components.count <= 2 else { return text }
        let digits = components.joined()
        guard !digits.isEmpty, digits.allSatisfy({ $0.isASCII && $0.isNumber }) else { return text }
        let significant = digits.drop(while: { $0 == "0" })
        guard !significant.isEmpty else { return "0" }
        let leadingZeros = digits.count - significant.count
        let offset = components[0].count - leadingZeros - 1
        let (exponent, overflow) = explicitExponent.addingReportingOverflow(offset)
        guard !overflow else { return text }
        let prefix = String(significant.prefix(4))
        let head = String(prefix.prefix(1))
        let tail = String(prefix.dropFirst())
        let coefficient = head + (tail.isEmpty ? "" : "." + tail)
        return (negative ? "-" : "") + coefficient + (significant.count > 4 ? "…" : "") + "e" + String(exponent)
    }

    private static func times(_ lower: String, _ upper: String) -> (String, String) {
        if lower.prefix(5) != upper.prefix(5) { return (String(lower.prefix(5)), String(upper.prefix(5))) }
        if lower.prefix(8) != upper.prefix(8) { return (String(lower.prefix(8)), String(upper.prefix(8))) }
        return (lower, upper)
    }

    private static func timestamp(_ text: String) -> (date: String, time: String, zone: String)? {
        guard text.range(of: #"^\d{4}-\d{2}-\d{2}(?:[T ]|$)"#, options: .regularExpression) != nil else {
            return nil
        }
        let date = String(text.prefix(10))
        var time = String(text.dropFirst(min(11, text.count)))
        var zone = ""
        if time.hasSuffix("Z") {
            time.removeLast()
            zone = "Z"
        } else if let start = time.range(of: #"[+-]\d{2}:\d{2}$"#, options: .regularExpression) {
            zone = String(time[start])
            time = String(time[..<start.lowerBound])
        }
        return (date, time, zone)
    }
}
