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
        // Selection colour depends on focus: redraw when the window's key
        // state changes.
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            NotificationCenter.default.addObserver(
                self, selector: #selector(tableScrolled(_:)), name: name, object: nil
            )
        }
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
        guard let table, let label else { return }
        let visible = table.rows(in: table.visibleRect)
        let selected = table.selectedRowIndexes
        // Selection is emphasized (accent) only while the table has focus,
        // matching how the table draws it.
        let emphasized = table.window?.isKeyWindow == true && table.window?.firstResponder === table
        let stripes = NSColor.alternatingContentBackgroundColors

        for row in visible.location..<(visible.location + visible.length) {
            var rowRect = convert(table.rect(ofRow: row), from: table)
            rowRect.origin.x = bounds.minX
            rowRect.size.width = bounds.width
            guard rowRect.intersects(dirtyRect) else { continue }

            // Continue the table's zebra stripes and selection into the gutter.
            let isSelected = selected.contains(row)
            if isSelected {
                (emphasized ? NSColor.selectedContentBackgroundColor
                    : NSColor.unemphasizedSelectedContentBackgroundColor).setFill()
            } else {
                stripes[row % stripes.count].setFill()
            }
            rowRect.fill()

            guard let text = label(row) else { continue }
            let color: NSColor = isSelected
                ? (emphasized ? .alternateSelectedControlTextColor : .secondaryLabelColor)
                : .tertiaryLabelColor
            let string = NSAttributedString(string: text, attributes: [
                .font: CellFonts.rowNumber, .foregroundColor: color,
            ])
            let size = string.size()
            string.draw(at: NSPoint(x: bounds.maxX - 8 - size.width, y: rowRect.midY - size.height / 2))
        }

        // A hairline between the numbers and the data.
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.maxX - 1, y: dirtyRect.minY, width: 1, height: dirtyRect.height).fill()
    }
}
