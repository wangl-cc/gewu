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

private func funnel(filled: Bool, color: NSColor) -> NSImage? {
    NSImage(
        systemSymbolName: filled ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle",
        accessibilityDescription: filled ? "Filtered" : "Filter"
    )?.withSymbolConfiguration(.init(hierarchicalColor: color))
}

/// The funnel shown on filtered columns.
let filterSymbol = funnel(filled: true, color: .controlAccentColor)
private let idleFilterSymbol = funnel(filled: false, color: .tertiaryLabelColor)
private let hoverFilterSymbol = funnel(filled: false, color: .secondaryLabelColor)

/// Table header cell with the column name on top and its type below,
/// and a funnel button on the right that opens the column's filter.
final class ColumnHeaderCell: NSTableHeaderCell {
    var subtitle = ""
    var filtered = false
    var rightAligned = false
    var sorted = false
    var sortAscending = true
    /// Data columns show the funnel button; the row-number column does not.
    var showsFunnel = false
    var funnelHovered = false

    private static let funnelSize: CGFloat = 14
    private static let sortIndicatorWidth: CGFloat = 14

    /// Where the funnel button sits inside a header cell's frame.
    static func funnelRect(in frame: NSRect) -> NSRect {
        NSRect(x: frame.maxX - 8 - funnelSize, y: frame.midY - funnelSize / 2,
               width: funnelSize, height: funnelSize)
    }

    /// The frame AppKit lays the sort arrow out in: everything left of the funnel.
    private func sortFrame(_ frame: NSRect) -> NSRect {
        guard showsFunnel else { return frame }
        var f = frame
        f.size.width -= Self.funnelSize + 6
        return f
    }

    override func sortIndicatorRect(forBounds rect: NSRect) -> NSRect {
        super.sortIndicatorRect(forBounds: sortFrame(rect))
    }

    /// The arrow is drawn by drawInterior, next to the funnel.
    override func drawSortIndicator(withFrame cellFrame: NSRect, in controlView: NSView,
                                    ascending: Bool, priority: Int) {}

    private static let ascendingArrow = sortArrow("chevron.up")
    private static let descendingArrow = sortArrow("chevron.down")

    private static func sortArrow(_ name: String) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 9, weight: .bold)
                .applying(.init(hierarchicalColor: .secondaryLabelColor)))
    }

    override func drawInterior(withFrame frame: NSRect, in controlView: NSView) {
        // AppKit reuses the last header cell to draw the empty area after the last column.
        guard !stringValue.isEmpty else { return }
        // Same inset as the data cells, so header and values line up.
        var rect = frame.insetBy(dx: 10, dy: 0)

        if showsFunnel {
            let image = filtered ? filterSymbol : (funnelHovered ? hoverFilterSymbol : idleFilterSymbol)
            image?.draw(in: Self.funnelRect(in: frame), from: .zero, operation: .sourceOver, fraction: 1,
                        respectFlipped: true, hints: nil)
            rect.size.width = Self.funnelRect(in: frame).minX - 4 - rect.minX
        }
        if sorted, let arrow = sortAscending ? Self.ascendingArrow : Self.descendingArrow {
            let size = arrow.size
            let arrowRect = NSRect(x: rect.maxX - size.width, y: frame.midY - size.height / 2,
                                   width: size.width, height: size.height)
            arrow.draw(in: arrowRect, from: .zero, operation: .sourceOver, fraction: 1,
                       respectFlipped: true, hints: nil)
            rect.size.width -= Self.sortIndicatorWidth
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

/// Header view that turns clicks on a column's funnel into a filter request
/// instead of a sort, and highlights the funnel under the mouse.
final class FilterHeaderView: NSTableHeaderView {
    var onFilterClick: ((Int) -> Void)?
    private var hovered: ColumnHeaderCell?

    private func funnelCell(at event: NSEvent) -> (Int, ColumnHeaderCell)? {
        guard let tableView else { return nil }
        let point = convert(event.locationInWindow, from: nil)
        let index = column(at: point)
        guard index >= 0, let cell = tableView.tableColumns[index].headerCell as? ColumnHeaderCell,
              cell.showsFunnel,
              ColumnHeaderCell.funnelRect(in: headerRect(ofColumn: index)).insetBy(dx: -4, dy: -4).contains(point)
        else { return nil }
        return (index, cell)
    }

    override func mouseDown(with event: NSEvent) {
        if let (index, _) = funnelCell(at: event) {
            onFilterClick?(index)
        } else {
            super.mouseDown(with: event)
        }
    }

    private var hoverArea: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self, userInfo: nil
        )
        addTrackingArea(area)
        hoverArea = area
    }

    override func mouseMoved(with event: NSEvent) {
        setHovered(funnelCell(at: event)?.1)
    }

    override func mouseExited(with event: NSEvent) {
        setHovered(nil)
    }

    private func setHovered(_ cell: ColumnHeaderCell?) {
        guard cell !== hovered else { return }
        hovered?.funnelHovered = false
        cell?.funnelHovered = true
        hovered = cell
        needsDisplay = true
    }
}

/// Row view that paints its own zebra stripe. NSTableView's built-in
/// alternating backgrounds repaint stripes across a huge area on every
/// resize frame for multi-million-row tables (most of the CPU when the
/// sidebar animates); painting per visible row costs only what's on screen.
final class StripedRowView: NSTableRowView {
    var odd = false {
        didSet { if odd != oldValue { needsDisplay = true } }
    }

    override func drawBackground(in dirtyRect: NSRect) {
        let colors = NSColor.alternatingContentBackgroundColors
        (odd && colors.count > 1 ? colors[1] : colors[0]).setFill()
        dirtyRect.fill()
    }
}

/// Data cell that can show a search match: a find-highlight behind the
/// whole value (numbers, dates) or behind matched text (via attributes).
final class MatchCellView: NSTableCellView {
    var wholeCellMatch = false {
        didSet { if wholeCellMatch != oldValue { needsDisplay = true } }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard wholeCellMatch, let field = textField else { return }
        // Hug the text, like Finder and Xcode find results.
        let text = field.attributedStringValue.size()
        let width = min(text.width + 8, bounds.width - 4)
        let x = field.alignment == .right ? bounds.maxX - 6 - width : bounds.minX + 6
        let rect = NSRect(x: x, y: bounds.midY - text.height / 2 - 1, width: width, height: text.height + 2)
        NSColor.findHighlightColor.setFill()
        NSBezierPath(roundedRect: rect, xRadius: 3, yRadius: 3).fill()
    }
}

/// Text with every case-insensitive occurrence of `query` highlighted.
func highlighted(_ text: String, query: String, font: NSFont) -> NSAttributedString {
    let result = NSMutableAttributedString(string: text, attributes: [
        .font: font, .foregroundColor: NSColor.labelColor,
    ])
    guard !query.isEmpty else { return result }
    var range = text.startIndex..<text.endIndex
    while let found = text.range(of: query, options: [.caseInsensitive], range: range) {
        result.addAttributes([
            .backgroundColor: NSColor.findHighlightColor,
            // The highlight is light in both appearances; keep text dark on it.
            .foregroundColor: NSColor.black,
        ], range: NSRange(found, in: text))
        range = found.upperBound..<text.endIndex
    }
    return result
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
