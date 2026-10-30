import AppKit
import SwiftUI

struct HistogramSelection: Equatable {
    var lower: Double
    var upper: Double
    var snapped = true
}

/// Preserve the existing chart's drag behavior inside SwiftUI forms.
struct HistogramChart: NSViewRepresentable {
    let histogram: Histogram
    @Binding var selection: HistogramSelection
    var interactive = true
    var labelFormatter: ((Double) -> String)?

    final class Coordinator {
        var histogram: Histogram?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> HistogramView { HistogramView() }

    func updateNSView(_ view: HistogramView, context: Context) {
        view.isInteractive = interactive
        view.labelFormatter = labelFormatter
        if let min = histogram.minLabel, let max = histogram.maxLabel {
            view.axisLabels = ChartLabels.endpoints(min, max)
            view.toolTip = "\(min) – \(max)"
        } else {
            view.axisLabels = nil
            view.toolTip = nil
        }
        if context.coordinator.histogram != histogram {
            view.setData(
                counts: histogram.counts, lo: histogram.lo ?? 0,
                hi: histogram.hi ?? 1, integer: histogram.integer)
            context.coordinator.histogram = histogram
        }
        view.setSelection(selection.lower, selection.upper, snapped: selection.snapped)
        view.onChange = { [weak view] in
            guard let view else { return }
            selection = HistogramSelection(
                lower: view.selection.0, upper: view.selection.1,
                snapped: view.isSnapped)
        }
        view.needsDisplay = true
    }

    static func dismantleNSView(_ view: HistogramView, coordinator: Coordinator) {
        view.onChange = nil
        view.labelFormatter = nil
        view.axisLabels = nil
    }
}

/// Bar chart of a histogram with a draggable selection.
///
/// Drag a handle to move that edge, drag inside the selection to move it,
/// or drag anywhere else to draw a new one. Double-click selects everything.
/// Dragging snaps to bin edges.
final class HistogramView: NSView {
    var onChange: (() -> Void)?
    /// When false, only shows the distribution: no handles, no dragging.
    var isInteractive = true
    /// Text for axis values; numbers by default.
    var labelFormatter: ((Double) -> String)?
    /// Exact source extrema supplied by a typed range histogram.
    var axisLabels: (String, String)?
    private(set) var selection: (Double, Double) = (0, 1)
    /// Whether the selection sits on bin edges (counts are then exact).
    private(set) var isSnapped = true

    private var counts: [UInt64] = []
    private var lo = 0.0, hi = 1.0
    private var integer = false
    private var maxCount: UInt64 = 1

    private enum Drag {
        case lower, upper
        case pan(startValue: Double, startSelection: (Double, Double))
        case create(anchor: Double)
    }
    private var drag: Drag?

    override var isFlipped: Bool { true }

    /// Start dragging on the first click, even before the popover is key.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// For integers `hi` is one past the largest value.
    func setData(counts: [UInt64], lo: Double, hi: Double, integer: Bool) {
        self.counts = counts
        self.lo = lo
        self.hi = hi
        self.integer = integer
        maxCount = max(1, counts.max() ?? 1)
        selection = (lo, hi)
        needsDisplay = true
    }

    func setSelection(_ a: Double, _ b: Double, snapped: Bool) {
        selection = (min(max(a, lo), hi), min(max(b, lo), hi))
        isSnapped = snapped
        needsDisplay = true
    }

    /// Rows in bins covered by the selection (partly covered bins count in
    /// proportion).
    func selectedCount() -> UInt64 {
        guard !counts.isEmpty, hi > lo else { return 0 }
        let width = (hi - lo) / Double(counts.count)
        var total = 0.0
        for (i, c) in counts.enumerated() {
            let b0 = lo + Double(i) * width
            let b1 = b0 + width
            let overlap = min(b1, selection.1) - max(b0, selection.0)
            if overlap > 0 { total += Double(c) * overlap / width }
        }
        return UInt64(total.rounded())
    }

    // MARK: Geometry

    private var plot: NSRect {
        // Side insets keep the edge handles (and their hit areas) inside the view.
        NSRect(x: bounds.minX + 11, y: bounds.minY + 4, width: bounds.width - 22, height: bounds.height - 22)
    }

    private func x(_ v: Double) -> CGFloat {
        guard hi > lo else { return plot.minX }
        return plot.minX + CGFloat((v - lo) / (hi - lo)) * plot.width
    }

    /// The bin edge nearest to `x`.
    private func snappedValue(_ x: CGFloat) -> Double {
        guard !counts.isEmpty else { return lo }
        let t = min(max(Double((x - plot.minX) / plot.width), 0), 1)
        let k = (t * Double(counts.count)).rounded()
        return lo + k / Double(counts.count) * (hi - lo)
    }

    /// The selection covers the whole axis (no range chosen).
    private var isEverything: Bool { selection.0 <= lo && selection.1 >= hi }

    private var binWidth: Double { counts.isEmpty ? 1 : (hi - lo) / Double(counts.count) }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        let p = plot
        NSColor.quaternarySystemFill.setFill()
        NSBezierPath(roundedRect: p.insetBy(dx: -1, dy: -2), xRadius: 4, yRadius: 4).fill()
        guard !counts.isEmpty else { return }

        let barWidth = p.width / CGFloat(counts.count)
        let gap: CGFloat = barWidth > 4 ? 1 : 0
        // Until a range is chosen, show the distribution in neutral grey.
        let everything = isEverything
        let selected = everything ? NSColor.secondaryLabelColor : NSColor.controlAccentColor
        let unselected = NSColor.tertiaryLabelColor
        for (i, c) in counts.enumerated() where c > 0 {
            let h = max(1, CGFloat(Double(c) / Double(maxCount)) * (p.height - 4))
            let r = NSRect(
                x: p.minX + CGFloat(i) * barWidth + gap / 2, y: p.maxY - h,
                width: barWidth - gap, height: h)
            let mid = lo + (Double(i) + 0.5) * binWidth
            (mid >= selection.0 && mid <= selection.1 ? selected : unselected).setFill()
            r.fill()
        }

        drawAxisLabels(p)
        guard isInteractive else { return }

        // Selection band and handles.
        let x0 = x(selection.0)
        let x1 = x(selection.1)
        if !everything {
            NSColor.controlAccentColor.withAlphaComponent(0.12).setFill()
            NSRect(x: x0, y: p.minY - 2, width: max(1, x1 - x0), height: p.height + 4).fill()
        }
        // Handles: a line with a grip, visible even before anything is chosen.
        let handleColor = everything ? NSColor.labelColor.withAlphaComponent(0.55) : NSColor.controlAccentColor
        for hx in [x0, x1] {
            handleColor.setFill()
            NSRect(x: hx - 1, y: p.minY - 2, width: 2, height: p.height + 4).fill()
            let grip = NSRect(x: hx - 5, y: p.midY - 13, width: 10, height: 26)
            NSBezierPath(roundedRect: grip, xRadius: 4, yRadius: 4).fill()
            NSColor.windowBackgroundColor.withAlphaComponent(0.8).setFill()
            for dx in [-1.5, 1.5] as [CGFloat] {
                NSRect(x: hx + dx - 0.5, y: grip.midY - 6, width: 1, height: 12).fill()
            }
        }
    }

    private func drawAxisLabels(_ p: NSRect) {
        // Axis labels: column min and max.
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 10, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let leftStyle = NSMutableParagraphStyle()
        leftStyle.lineBreakMode = .byTruncatingTail
        let rightStyle = NSMutableParagraphStyle()
        rightStyle.lineBreakMode = .byTruncatingHead
        rightStyle.alignment = .right
        var leftAttrs = attrs
        var rightAttrs = attrs
        leftAttrs[.paragraphStyle] = leftStyle
        rightAttrs[.paragraphStyle] = rightStyle
        let left = NSAttributedString(string: axisLabels?.0 ?? axisLabel(lo), attributes: leftAttrs)
        let right = NSAttributedString(
            string: axisLabels?.1 ?? axisLabel(integer ? hi - 1 : hi), attributes: rightAttrs)
        let width = max(0, (p.width - 8) / 2)
        left.draw(in: NSRect(x: p.minX, y: p.maxY + 4, width: width, height: 15))
        right.draw(in: NSRect(x: p.maxX - width, y: p.maxY + 4, width: width, height: 15))
    }

    private func axisLabel(_ v: Double) -> String {
        if let labelFormatter { return labelFormatter(v) }
        if v == v.rounded(), abs(v) < 1e15 {
            return NumberFormatter.localizedString(from: NSNumber(value: Int64(v)), number: .decimal)
        }
        return String(format: "%.6g", v)
    }

    // MARK: Mouse

    /// Handles win over panning and new selections within this distance.
    private static let handleReach: CGFloat = 10

    private func handle(at point: NSPoint) -> Drag? {
        let x0 = x(selection.0)
        let x1 = x(selection.1)
        let d0 = abs(point.x - x0)
        let d1 = abs(point.x - x1)
        guard min(d0, d1) <= Self.handleReach else { return nil }
        // When both are in reach (a narrow selection), take the nearer one.
        return d1 < d0 || (d1 == d0 && point.x > x1) ? .upper : .lower
    }

    /// The chart handles its own drags; never let them move the window.
    override var mouseDownCanMoveWindow: Bool { false }

    private var moved = false

    override func mouseDown(with event: NSEvent) {
        guard isInteractive else { return }
        let point = convert(event.locationInWindow, from: nil)
        // A double click resets, but only on mouse up without movement:
        // trackpad "tap then drag" also arrives as a second click.
        moved = false
        if let h = handle(at: point) {
            drag = h
        } else if !isEverything, point.x > x(selection.0), point.x < x(selection.1) {
            drag = .pan(startValue: snappedValue(point.x), startSelection: selection)
        } else {
            // Outside the selection, or nothing chosen yet: draw a new range.
            drag = .create(anchor: snappedValue(point.x))
        }
    }

    override func mouseDragged(with event: NSEvent) {
        guard let drag else { return }
        moved = true
        if case .pan = drag { NSCursor.closedHand.set() }
        let v = snappedValue(convert(event.locationInWindow, from: nil).x)
        var (a, b) = selection
        switch drag {
        case .lower: a = min(v, b - binWidth)
        case .upper: b = max(v, a + binWidth)
        case .pan(let start, let sel):
            let width = sel.1 - sel.0
            a = min(max(sel.0 + (v - start), lo), hi - width)
            b = a + width
        case .create(let anchor):
            a = min(anchor, v)
            b = max(anchor, v)
            if b - a < binWidth { b = min(hi, a + binWidth) }
        }
        setSelection(a, b, snapped: true)
        onChange?()
    }

    override func mouseUp(with event: NSEvent) {
        drag = nil
        if !moved, event.clickCount >= 2 {
            setSelection(lo, hi, snapped: true)
            onChange?()
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(
            NSTrackingArea(
                rect: .zero, options: [.mouseMoved, .activeAlways, .inVisibleRect],
                owner: self, userInfo: nil))
    }

    override func mouseMoved(with event: NSEvent) {
        guard isInteractive else { return }
        let point = convert(event.locationInWindow, from: nil)
        if handle(at: point) != nil {
            NSCursor.resizeLeftRight.set()
        } else if !isEverything, point.x > x(selection.0), point.x < x(selection.1) {
            NSCursor.openHand.set()
        } else {
            NSCursor.crosshair.set()
        }
    }
}
