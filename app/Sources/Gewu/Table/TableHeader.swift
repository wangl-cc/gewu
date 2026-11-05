import AppKit

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
    var textSize: CGFloat = 12
    var subtitle = ""
    var filtered = false
    var inspected = false
    var rightAligned = false
    var sorted = false
    var sortAscending = true
    var sortPriority = 0
    /// Data columns show the funnel button; the row-number column does not.
    var showsFunnel = false
    var funnelHovered = false

    private static let funnelSize: CGFloat = 14
    private static let sortIndicatorWidth: CGFloat = 14

    /// Where the funnel button sits inside a header cell's frame.
    static func funnelRect(in frame: NSRect) -> NSRect {
        NSRect(
            x: frame.maxX - 8 - funnelSize, y: frame.midY - funnelSize / 2,
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
    override func drawSortIndicator(
        withFrame cellFrame: NSRect, in controlView: NSView,
        ascending: Bool, priority: Int
    ) {}

    private static let ascendingArrow = sortArrow("chevron.up")
    private static let descendingArrow = sortArrow("chevron.down")

    private static func sortArrow(_ name: String) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(
                .init(pointSize: 9, weight: .bold)
                    .applying(.init(hierarchicalColor: .secondaryLabelColor)))
    }

    override func draw(withFrame frame: NSRect, in controlView: NSView) {
        // NSTableHeaderCell's bezel adds two bottom edges. The header view
        // owns the single separator; cells draw only their fill and contents.
        (isHighlighted ? NSColor.selectedControlColor : NSColor.controlBackgroundColor).setFill()
        frame.fill()
        if inspected, !stringValue.isEmpty {
            NSColor.controlAccentColor.withAlphaComponent(0.08).setFill()
            frame.fill()
            NSColor.controlAccentColor.setFill()
            NSRect(x: frame.minX, y: frame.maxY - 2, width: frame.width, height: 2).fill()
        }
        drawInterior(withFrame: frame, in: controlView)

        // AppKit draws the trailing empty area with a copy of the last cell
        // whose title is cleared. Do not add a divider to that filler cell.
        if !stringValue.isEmpty || !showsFunnel {
            let pixel = 1 / (controlView.window?.backingScaleFactor ?? 1)
            NSColor.separatorColor.setFill()
            NSRect(
                x: frame.maxX - pixel, y: frame.minY + 8,
                width: pixel, height: max(0, frame.height - 16)
            ).fill()
        }
    }

    override func drawInterior(withFrame frame: NSRect, in controlView: NSView) {
        // AppKit reuses the last header cell to draw the empty area after the last column.
        guard !stringValue.isEmpty else { return }
        // Same inset as the data cells, so header and values line up.
        var rect = frame.insetBy(dx: 10, dy: 0)

        if showsFunnel {
            let image = filtered ? filterSymbol : (funnelHovered ? hoverFilterSymbol : idleFilterSymbol)
            image?.draw(
                in: Self.funnelRect(in: frame), from: .zero, operation: .sourceOver, fraction: 1,
                respectFlipped: true, hints: nil)
            rect.size.width = Self.funnelRect(in: frame).minX - 4 - rect.minX
        }
        if sorted {
            let badge = NSAttributedString(
                string: "\(sortAscending ? "↑" : "↓")\(sortPriority)",
                attributes: [
                    .font: NSFont.monospacedDigitSystemFont(ofSize: 10, weight: .semibold),
                    .foregroundColor: NSColor.secondaryLabelColor,
                ])
            let size = badge.size()
            badge.draw(at: NSPoint(x: rect.maxX - size.width, y: frame.midY - size.height / 2))
            rect.size.width -= size.width + 5
        }
        guard rect.width > 4 else { return }

        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        paragraph.alignment = rightAligned ? .right : .left

        let name = NSAttributedString(
            string: stringValue,
            attributes: [
                .font: NSFont.systemFont(ofSize: textSize, weight: .semibold),
                .foregroundColor: filtered ? NSColor.controlAccentColor : NSColor.labelColor,
                .paragraphStyle: paragraph,
            ])
        let nameHeight = ceil(textSize * 1.25)
        if subtitle.isEmpty {
            name.draw(
                with: NSRect(x: rect.minX, y: frame.midY - nameHeight / 2, width: rect.width, height: nameHeight),
                options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
            return
        }
        let typeHeight = ceil((textSize - 2) * 1.3)
        let top = frame.minY + (frame.height - nameHeight - typeHeight) / 2
        name.draw(
            with: NSRect(x: rect.minX, y: top, width: rect.width, height: nameHeight),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        let type = NSAttributedString(
            string: subtitle,
            attributes: [
                .font: NSFont.systemFont(ofSize: textSize - 2),
                .foregroundColor: NSColor.secondaryLabelColor,
                .paragraphStyle: paragraph,
            ])
        type.draw(
            with: NSRect(x: rect.minX, y: top + nameHeight, width: rect.width, height: typeHeight),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }
}

/// Header view that turns clicks on a column's funnel into a filter request
/// instead of a sort, and highlights the funnel under the mouse.
final class FilterHeaderView: NSTableHeaderView {
    var onFilterClick: ((Int) -> Void)?
    var columnMenu: ((Int) -> NSMenu?)?
    /// Any other click on a column header (it also sorts, as usual).
    private(set) var sortingMultiple = false
    private var hovered: ColumnHeaderCell?

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        let pixel = 1 / (window?.backingScaleFactor ?? 1)
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.minX, y: bounds.maxY - pixel, width: bounds.width, height: pixel).fill()
    }

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

    override func menu(for event: NSEvent) -> NSMenu? {
        columnMenu?(column(at: convert(event.locationInWindow, from: nil)))
    }

    override func mouseDown(with event: NSEvent) {
        if let (index, _) = funnelCell(at: event) {
            onFilterClick?(index)
        } else {
            // Keep AppKit's click/drag/divider tracking intact. Shift means
            // multi-sort for a completed click, not native column selection.
            sortingMultiple = event.modifierFlags.contains(.shift)
            defer { sortingMultiple = false }
            // Ordinary drags need the original event; reconstructing it loses
            // native tracking information. Only normalize Shift-click sorting.
            guard sortingMultiple else {
                super.mouseDown(with: event)
                return
            }
            let tracked =
                NSEvent.mouseEvent(
                    with: event.type, location: event.locationInWindow,
                    modifierFlags: event.modifierFlags.subtracting(.shift), timestamp: event.timestamp,
                    windowNumber: event.windowNumber, context: nil, eventNumber: event.eventNumber,
                    clickCount: event.clickCount, pressure: event.pressure) ?? event
            super.mouseDown(with: tracked)
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
