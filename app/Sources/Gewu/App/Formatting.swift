import Foundation

func formatCount(_ n: UInt64) -> String {
    NumberFormatter.localizedString(from: NSNumber(value: n), number: .decimal)
}

/// A short, readable label for an Arrow type, e.g. "Utf8" -> "string",
/// "Timestamp(Microsecond, None)" -> "timestamp", "Decimal128(10, 2)" -> "decimal(10,2)".
func shortTypeName(_ typeName: String) -> String {
    if typeName.hasPrefix("Decimal"), let open = typeName.firstIndex(of: "(") {
        return "decimal" + typeName[open...].replacingOccurrences(of: " ", with: "")
    }
    let base = String(typeName.prefix { $0 != "(" && $0 != "<" })
    switch base {
    case "Utf8", "LargeUtf8", "Utf8View": return "string"
    case "Binary", "LargeBinary", "BinaryView", "FixedSizeBinary": return "binary"
    case "Boolean": return "bool"
    case "Date32", "Date64": return "date"
    case "Time32", "Time64": return "time"
    case "List", "LargeList", "FixedSizeList", "ListView", "LargeListView": return "list"
    default: return base.lowercased()
    }
}
