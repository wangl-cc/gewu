import AppKit

/// Range filter for number, date and time columns: a histogram of the
/// column with a draggable selection, plus min/max fields kept in sync.
/// Dates and times are handled as their integer physical value; the Rust
/// core formats and parses them so they read exactly like the table.
final class RangeFilterController: NSViewController, NSTextFieldDelegate {
    private let onApply: ([Filter]) -> Void
    private let doc: GewuDocument
    private let column: UInt32
    private let info: ColumnInfo
    private let temporal: Bool
    private let state: ViewState
    private let initialRange: (Bound?, Bound?)
    private var nullMode: NullMode

    private var histogram: Histogram?
    private let chart = HistogramView()
    private let minField = NSTextField()
    private let maxField = NSTextField()
    private let summary = noteLabel("Loading…")
    private let hint = noteLabel("Drag to select a range · Drag the handles or the selection · Double-click to reset")
    private lazy var nulls = NullMode.popUp(nullCount: nil, target: self, action: #selector(nullModeChanged(_:)))

    private static let bins: UInt32 = 60

    init(doc: GewuDocument, column: UInt32, info: ColumnInfo, temporal: Bool, state: ViewState,
         current: [Filter], onApply: @escaping ([Filter]) -> Void)
    {
        self.doc = doc
        self.column = column
        self.info = info
        self.temporal = temporal
        self.state = state
        self.onApply = onApply
        var range: (Bound?, Bound?) = (nil, nil)
        for case let .range(_, min, max) in current {
            range = (min, max)
        }
        initialRange = range
        nullMode = NullMode(current)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 420, height: 290))

        let title = NSTextField(labelWithString: info.name)
        title.font = .systemFont(ofSize: 13, weight: .semibold)
        let type = noteLabel(shortTypeName(info.typeName))

        chart.onChange = { [weak self] in self?.selectionChanged(fromChart: true) }
        chart.labelFormatter = { [weak self] v in self?.format(v) ?? "" }

        for (field, placeholder) in [(minField, "Min"), (maxField, "Max")] {
            field.placeholderString = placeholder
            field.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
            field.alignment = .right
            field.delegate = self
            field.target = self
            field.action = #selector(fieldCommitted(_:))
        }
        let dash = NSTextField(labelWithString: "–")
        let fields = NSStackView(views: [minField, dash, maxField])
        fields.orientation = .horizontal
        fields.spacing = 6
        minField.widthAnchor.constraint(equalTo: maxField.widthAnchor).isActive = true

        let reset = NSButton(title: "Reset", target: self, action: #selector(reset(_:)))
        let ok = NSButton(title: "OK", target: self, action: #selector(apply(_:)))
        ok.keyEquivalent = "\r"
        let buttons = NSStackView(views: [nulls, NSView(), reset, ok])
        buttons.orientation = .horizontal

        let heading = NSStackView(views: [title, type])
        heading.orientation = .horizontal
        heading.alignment = .firstBaseline
        heading.spacing = 6

        hint.textColor = .tertiaryLabelColor
        for v in [heading, chart, hint, fields, summary, buttons] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(v)
        }
        let inset: CGFloat = 14
        NSLayoutConstraint.activate([
            heading.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            heading.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: inset),

            chart.topAnchor.constraint(equalTo: heading.bottomAnchor, constant: 8),
            chart.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: inset),
            chart.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -inset),
            chart.heightAnchor.constraint(equalToConstant: 140),

            hint.topAnchor.constraint(equalTo: chart.bottomAnchor, constant: 2),
            hint.leadingAnchor.constraint(equalTo: chart.leadingAnchor),
            hint.trailingAnchor.constraint(equalTo: chart.trailingAnchor),

            fields.topAnchor.constraint(equalTo: hint.bottomAnchor, constant: 8),
            fields.leadingAnchor.constraint(equalTo: chart.leadingAnchor),
            fields.trailingAnchor.constraint(equalTo: chart.trailingAnchor),

            summary.topAnchor.constraint(equalTo: fields.bottomAnchor, constant: 6),
            summary.leadingAnchor.constraint(equalTo: chart.leadingAnchor),
            summary.trailingAnchor.constraint(equalTo: chart.trailingAnchor),

            buttons.topAnchor.constraint(equalTo: summary.bottomAnchor, constant: 10),
            buttons.leadingAnchor.constraint(equalTo: chart.leadingAnchor),
            buttons.trailingAnchor.constraint(equalTo: chart.trailingAnchor),
            buttons.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -12),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        let doc = doc, column = column, state = state, bins = Self.bins
        Task {
            do {
                let h = try await Task.detached {
                    try doc.histogram(column: column, state: state, bins: bins)
                }.value
                show(h)
            } catch {
                summary.stringValue = error.localizedDescription
            }
        }
    }

    private func show(_ h: Histogram) {
        histogram = h
        nulls.item(at: 2)?.title = "Only nulls (\(formatCount(h.nullCount)))"
        nulls.selectItem(at: nullMode.rawValue)
        guard let lo = h.lo, let hi = h.hi else {
            summary.stringValue = "No values to plot"
            return
        }
        chart.setData(counts: h.counts, lo: lo, hi: hi, integer: h.integer)
        // Start from the current filter, or the whole range.
        var a = lo, b = hi
        if let min = initialRange.0 { a = Swift.max(lo, value(min)) }
        if let max = initialRange.1 { b = Swift.min(hi, value(max) + (h.integer ? 1 : 0)) }
        chart.setSelection(a, Swift.max(a, b), snapped: initialRange == (nil, nil))
        selectionChanged(fromChart: false)
    }

    private func value(_ b: Bound) -> Double {
        switch b {
        case let .int(v): Double(v)
        case let .uInt(v): Double(v)
        case let .float(v): v
        }
    }

    // MARK: - Selection

    private var isInteger: Bool { histogram?.integer ?? false }

    /// The selection as an inclusive range of values.
    /// The chart works with bin edges `[a, b)`; for integers that is
    /// `a ... b - 1`.
    private func inclusive() -> (Double, Double) {
        let (a, b) = chart.selection
        return isInteger ? (a.rounded(.up), b.rounded(.up) - 1) : (a, b)
    }

    private func isFullRange() -> Bool {
        guard let lo = histogram?.lo, let hi = histogram?.hi else { return true }
        return chart.selection.0 <= lo && chart.selection.1 >= hi
    }

    private func format(_ v: Double) -> String {
        if temporal, let text = try? doc.formatValue(column: column, value: Int64(v)) { return text }
        if isInteger { return String(Int64(v)) }
        return String(format: "%.7g", v)
    }

    /// A typed bound: a plain number, or a date/time in the column's format.
    private func parse(_ text: String) -> Double? {
        if !temporal, let v = Double(text) { return v }
        return (try? doc.parseValue(column: column, text: text)).map(value)
    }

    private func selectionChanged(fromChart: Bool) {
        guard let h = histogram, h.lo != nil else { return }
        let (a, b) = inclusive()
        minField.stringValue = format(a)
        maxField.stringValue = format(b)

        let selected = chart.selectedCount()
        let base = h.total - h.nullCount - h.nanCount
        // Counts are exact only when the selection sits on bin edges.
        let exact = chart.isSnapped
        let prefix = exact ? "" : "≈ "
        summary.stringValue = isFullRange()
            ? "All \(formatCount(base)) values"
            : "\(prefix)\(formatCount(selected)) of \(formatCount(base)) values selected"
    }

    @objc private func fieldCommitted(_ sender: NSTextField) {
        guard let lo = histogram?.lo, let hi = histogram?.hi else { return }
        var (a, b) = chart.selection
        let text = sender.stringValue.trimmingCharacters(in: .whitespaces)
        if sender === minField {
            a = parse(text).map { Swift.max(lo, $0) } ?? lo
        } else {
            // Inclusive max: for integers the bin edge is one past it.
            b = parse(text).map { Swift.min(hi, isInteger ? $0.rounded(.down) + 1 : $0) } ?? hi
        }
        chart.setSelection(a, Swift.max(a, b), snapped: false)
        selectionChanged(fromChart: false)
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        if let field = obj.object as? NSTextField { fieldCommitted(field) }
    }

    @objc private func reset(_ sender: Any?) {
        guard let lo = histogram?.lo, let hi = histogram?.hi else { return }
        chart.setSelection(lo, hi, snapped: true)
        nullMode = .include
        nulls.selectItem(at: 0)
        selectionChanged(fromChart: false)
    }

    @objc private func nullModeChanged(_ sender: NSPopUpButton) {
        nullMode = NullMode(rawValue: sender.indexOfSelectedItem) ?? .include
    }

    @objc private func apply(_ sender: Any?) {
        if nullMode == .only, let f = nullMode.filter(column: column) {
            onApply([f])
            return
        }
        var filters: [Filter] = []
        if histogram?.lo != nil, !isFullRange() {
            let (a, b) = inclusive()
            let atLo = chart.selection.0 <= (histogram?.lo ?? a)
            let atHi = chart.selection.1 >= (histogram?.hi ?? b)
            filters.append(.range(column: column, min: atLo ? nil : bound(a), max: atHi ? nil : bound(b)))
        } else if let f = nullMode.filter(column: column) {
            // A range already drops nulls, so "hide nulls" only matters alone.
            filters.append(f)
        }
        onApply(filters)
    }

    private func bound(_ v: Double) -> Bound {
        if isInteger, let i = Int64(exactly: v) { return .int(value: i) }
        return .float(value: v)
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
            let b0 = lo + Double(i) * width, b1 = b0 + width
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
            let r = NSRect(x: p.minX + CGFloat(i) * barWidth + gap / 2, y: p.maxY - h,
                           width: barWidth - gap, height: h)
            let mid = lo + (Double(i) + 0.5) * binWidth
            (mid >= selection.0 && mid <= selection.1 ? selected : unselected).setFill()
            r.fill()
        }

        drawAxisLabels(p)
        guard isInteractive else { return }

        // Selection band and handles.
        let x0 = x(selection.0), x1 = x(selection.1)
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
        let left = NSAttributedString(string: axisLabel(lo), attributes: attrs)
        let right = NSAttributedString(string: axisLabel(integer ? hi - 1 : hi), attributes: attrs)
        left.draw(at: NSPoint(x: p.minX, y: p.maxY + 4))
        right.draw(at: NSPoint(x: p.maxX - right.size().width, y: p.maxY + 4))
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
        let x0 = x(selection.0), x1 = x(selection.1)
        let d0 = abs(point.x - x0), d1 = abs(point.x - x1)
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
        case let .pan(start, sel):
            let width = sel.1 - sel.0
            a = min(max(sel.0 + (v - start), lo), hi - width)
            b = a + width
        case let .create(anchor):
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
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .activeAlways, .inVisibleRect],
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
