import AppKit

/// How a column's values are displayed, derived from its Arrow type name.
enum ColumnKind {
    case number, temporal, text

    init(typeName: String) {
        let t = typeName
        if ["Int", "UInt", "Float", "Decimal"].contains(where: t.hasPrefix) {
            self = .number
        } else if ["Date", "Time", "Duration", "Interval"].contains(where: t.hasPrefix) {
            self = .temporal
        } else {
            self = .text
        }
    }

    var alignment: NSTextAlignment { self == .text ? .left : .right }
    var font: NSFont { self == .text ? CellFonts.text : CellFonts.digits }
}

enum CellFonts {
    static let text = NSFont.systemFont(ofSize: 12)
    static let digits = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .regular)
    static let rowNumber = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular)
    static let null: NSFont = {
        let base = NSFont.systemFont(ofSize: 12)
        return NSFont(descriptor: base.fontDescriptor.withSymbolicTraits(.italic), size: 12) ?? base
    }()
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

/// The funnel shown on filtered columns.
let filterSymbol: NSImage? = NSImage(
    systemSymbolName: "line.3.horizontal.decrease.circle.fill",
    accessibilityDescription: "Filtered"
)?.withSymbolConfiguration(.init(hierarchicalColor: .controlAccentColor))

/// Table header cell with the column name on top and its type below.
final class ColumnHeaderCell: NSTableHeaderCell {
    var subtitle = ""
    var filtered = false
    var rightAligned = false
    var sorted = false

    /// Room kept on the right for the sort arrow.
    private static let sortIndicatorWidth: CGFloat = 14

    override func drawInterior(withFrame frame: NSRect, in controlView: NSView) {
        // AppKit reuses the last header cell to draw the empty area after the last column.
        guard !stringValue.isEmpty else { return }
        // Same inset as the data cells, so header and values line up.
        var rect = frame.insetBy(dx: 10, dy: 0)
        if sorted { rect.size.width -= Self.sortIndicatorWidth }

        if filtered, let image = filterSymbol {
            let size: CGFloat = 13
            let imageRect = NSRect(x: rect.maxX - size, y: frame.midY - size / 2, width: size, height: size)
            image.draw(in: imageRect, from: .zero, operation: .sourceOver, fraction: 1,
                       respectFlipped: true, hints: nil)
            rect.size.width -= size + 4
        }
        guard rect.width > 4 else { return }

        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        paragraph.alignment = rightAligned ? .right : .left

        let name = NSAttributedString(string: stringValue, attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold),
            .foregroundColor: filtered ? NSColor.controlAccentColor : NSColor.labelColor,
            .paragraphStyle: paragraph,
        ])
        let nameHeight: CGFloat = 15
        if subtitle.isEmpty {
            name.draw(with: NSRect(x: rect.minX, y: frame.midY - nameHeight / 2, width: rect.width, height: nameHeight),
                      options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
            return
        }
        let typeHeight: CGFloat = 13
        let top = frame.minY + (frame.height - nameHeight - typeHeight) / 2
        name.draw(with: NSRect(x: rect.minX, y: top, width: rect.width, height: nameHeight),
                  options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        let type = NSAttributedString(string: subtitle, attributes: [
            .font: NSFont.systemFont(ofSize: 10),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: paragraph,
        ])
        type.draw(with: NSRect(x: rect.minX, y: top + nameHeight, width: rect.width, height: typeHeight),
                  options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }
}

/// Cell for the row-number column: tinted background and a divider on the right.
final class RowNumberCellView: NSTableCellView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor.quaternarySystemFill.setFill()
        bounds.fill()
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.maxX - 1, y: bounds.minY, width: 1, height: bounds.height).fill()
    }
}

/// A thin horizontal bar showing a fraction, used in the filter list.
final class ProportionBar: NSView {
    var fraction: CGFloat = 0 { didSet { needsDisplay = true } }

    override func draw(_ dirtyRect: NSRect) {
        let track = bounds.insetBy(dx: 0, dy: (bounds.height - 4) / 2)
        NSColor.quaternarySystemFill.setFill()
        NSBezierPath(roundedRect: track, xRadius: 2, yRadius: 2).fill()
        guard fraction > 0 else { return }
        var fill = track
        fill.size.width = max(2, track.width * min(fraction, 1))
        NSColor.controlAccentColor.withAlphaComponent(0.7).setFill()
        NSBezierPath(roundedRect: fill, xRadius: 2, yRadius: 2).fill()
    }
}
