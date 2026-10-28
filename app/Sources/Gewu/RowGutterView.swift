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

    private var fittedWidth: CGFloat = 40
    private var shown = true

    /// Fit the widest number expected.
    func fit(largest: UInt64) {
        let digits = String(repeating: "8", count: String(largest).count)
        let width = (digits as NSString).size(withAttributes: [.font: CellFonts.rowNumber]).width
        fittedWidth = max(28, ceil(width) + 16)
        setShown(shown)
    }

    /// Hidden collapses the gutter so the table takes its place.
    func setShown(_ shown: Bool) {
        self.shown = shown
        isHidden = !shown
        widthConstraint.constant = shown ? fittedWidth : 0
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.maxX - 1, y: dirtyRect.minY, width: 1, height: dirtyRect.height).fill()

        guard let table, let label else { return }
        let visible = table.rows(in: table.visibleRect)
        guard visible.length > 0 else { return }
        // Don't draw numbers under the floating table header.
        var clip = bounds
        if let header = table.headerView {
            let headerBottom = convert(header.bounds, from: header).maxY
            clip.origin.y = headerBottom
            clip.size.height = max(0, bounds.maxY - headerBottom)
        }
        NSBezierPath(rect: clip).setClip()

        let selected = table.selectedRowIndexes
        for row in visible.location..<(visible.location + visible.length) {
            guard let text = label(row) else { continue }
            let rowRect = convert(table.rect(ofRow: row), from: table)
            guard rowRect.intersects(dirtyRect) else { continue }
            let attrs: [NSAttributedString.Key: Any] = [
                .font: CellFonts.rowNumber,
                .foregroundColor: selected.contains(row) ? NSColor.controlAccentColor : NSColor.tertiaryLabelColor,
            ]
            let string = NSAttributedString(string: text, attributes: attrs)
            let size = string.size()
            string.draw(at: NSPoint(x: bounds.maxX - 8 - size.width, y: rowRect.midY - size.height / 2))
        }
    }
}
