import AppKit

/// Row numbers in a gutter beside the table, like Xcode's line numbers.
/// A separate view to the table's left: it follows the table's vertical
/// scrolling but not its sideways scrolling, and has no stripes or cell
/// borders of its own.
final class RowGutterView: NSView {
    /// The number shown for a display row, or `nil` while it is loading.
    var label: ((Int) -> String?)?

    private weak var table: NSTableView?
    private lazy var widthConstraint = widthAnchor.constraint(equalToConstant: 40)

    init(table: NSTableView) {
        self.table = table
        super.init(frame: .zero)
        widthConstraint.isActive = true
        // Redraw as the table scrolls.
        if let clip = table.enclosingScrollView?.contentView {
            clip.postsBoundsChangedNotifications = true
            NotificationCenter.default.addObserver(
                self, selector: #selector(tableScrolled(_:)),
                name: NSView.boundsDidChangeNotification, object: clip
            )
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }

    @objc private func tableScrolled(_ note: Notification) {
        needsDisplay = true
    }

    private(set) var fittedWidth: CGFloat = 40
    private var shown = true

    /// Fit the widest number expected.
    func fit(largest: UInt64) {
        let digits = String(repeating: "8", count: String(largest).count)
        let width = (digits as NSString).size(withAttributes: [.font: CellFonts.rowNumber]).width
        fittedWidth = max(24, ceil(width) + 12)
        setShown(shown)
    }

    /// Hidden collapses the gutter so the table takes its place.
    func setShown(_ shown: Bool) {
        self.shown = shown
        isHidden = !shown
        widthConstraint.constant = shown ? fittedWidth : 0
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let table else { return }
        // The header band is left to the table header (the corner cell of
        // the empty first column shows through); below it, the table's own
        // background, without stripes.
        let headerHeight = table.headerView?.frame.height ?? 0
        let rows = NSRect(x: 0, y: headerHeight, width: bounds.width, height: max(0, bounds.height - headerHeight))
        NSBezierPath(rect: rows).setClip()
        table.backgroundColor.setFill()
        rows.intersection(dirtyRect).fill()
        // Hairline between the numbers and the data.
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.maxX - 1, y: rows.minY, width: 1, height: rows.height).fill()

        guard let label else { return }
        let visible = table.rows(in: table.visibleRect)
        guard visible.length > 0 else { return }
        let selected = table.selectedRowIndexes
        for row in visible.location..<(visible.location + visible.length) {
            guard let text = label(row) else { continue }
            let rowRect = convert(table.rect(ofRow: row), from: table)
            guard rowRect.intersects(dirtyRect) else { continue }
            let string = NSAttributedString(string: text, attributes: [
                .font: CellFonts.rowNumber,
                .foregroundColor: selected.contains(row) ? NSColor.controlAccentColor : NSColor.tertiaryLabelColor,
            ])
            let size = string.size()
            string.draw(at: NSPoint(x: bounds.maxX - 6 - size.width, y: rowRect.midY - size.height / 2))
        }
    }

    /// Only the rows part takes clicks; the header band belongs to the table.
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let table, let hit = super.hitTest(point) else { return nil }
        let local = convert(point, from: superview)
        return local.y < (table.headerView?.frame.height ?? 0) ? nil : hit
    }
}
