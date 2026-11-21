import AppKit
import GewuKit

/// The app chooses styling from the coarse value category reported by Core.
extension ValueCategory {
    var alignment: NSTextAlignment { self == .text ? .left : .right }
}

struct CellFonts {
    let text: NSFont
    let digits: NSFont
    let rowNumber: NSFont
    let null: NSFont

    init(size: CGFloat = 12) {
        text = .systemFont(ofSize: size)
        digits = .monospacedDigitSystemFont(ofSize: size, weight: .regular)
        rowNumber = .monospacedDigitSystemFont(ofSize: max(9, size - 2), weight: .regular)
        null = NSFont(descriptor: text.fontDescriptor.withSymbolicTraits(.italic), size: size) ?? text
    }

    func font(for kind: ValueCategory) -> NSFont { kind == .text ? text : digits }
}

/// Row view that colors its own zebra stripe. NSTableView's built-in
/// alternating backgrounds repaint stripes across a huge area on every
/// resize frame for multi-million-row tables (most of the CPU when the
/// sidebar animates); assigning a color per visible row limits that work.
final class StripedRowView: NSTableRowView {
    var odd = false {
        didSet { updateStripe() }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateStripe()
    }

    private func updateStripe() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let base = NSColor.controlBackgroundColor
            layer?.backgroundColor = (odd ? base.blended(withFraction: 0.025, of: .labelColor) ?? base : base).cgColor
        }
    }
}

/// Data cell that can show a search match: a find-highlight behind the
/// whole value (numbers, dates) or behind matched text (via attributes).
final class MatchCellView: NSTableCellView {
    /// The rows this cell's content came from.
    weak var boundRows: DisplayRows?
    var isLoading = false
    private(set) var wholeCellMatch = false
    private(set) var currentMatch = false
    private var value = ""
    private var query: String?
    private(set) var muted = false
    private var font = NSFont.systemFont(ofSize: 12)
    private var alignment = NSTextAlignment.left

    private struct TextAppearance: Equatable {
        let value: String
        let query: String?
        let font: NSFont
        let alignment: NSTextAlignment
        let foreground: NSColor
    }
    private var rendered: TextAppearance?
    private var usesAttributedText = false

    func configure(
        text: String, font: NSFont, alignment: NSTextAlignment,
        query: String? = nil, wholeMatch: Bool = false, current: Bool = false, muted: Bool = false
    ) {
        value = text
        self.query = query
        let decorationChanged = wholeCellMatch != wholeMatch || currentMatch != current
        wholeCellMatch = wholeMatch
        currentMatch = current
        self.muted = muted
        self.font = font
        self.alignment = alignment
        refreshAppearance()
        if decorationChanged { needsDisplay = true }
    }

    /// Find decoration does not require another read of the cell's value.
    func updateFind(query: String?, wholeMatch: Bool, current: Bool) {
        configure(
            text: value, font: font, alignment: alignment, query: query,
            wholeMatch: wholeMatch, current: current, muted: muted)
    }

    func updateStyle(font: NSFont, alignment: NSTextAlignment) {
        configure(
            text: value, font: font, alignment: alignment, query: query,
            wholeMatch: wholeCellMatch, current: currentMatch, muted: muted)
    }

    /// The cell is one accessibility element carrying its text, rather than
    /// a cell wrapping a separate text element.
    override func accessibilityLabel() -> String? { value }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet {
            guard oldValue != backgroundStyle else { return }
            refreshAppearance()
            needsDisplay = true
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        rendered = nil
        refreshAppearance()
        needsDisplay = true
    }

    func refreshAppearance() {
        guard let field = textField else { return }
        let selected = backgroundStyle == .emphasized && window?.isKeyWindow != false
        let foreground: NSColor =
            wholeCellMatch
            ? .black
            : (selected ? .alternateSelectedControlTextColor : (muted ? .tertiaryLabelColor : .labelColor))
        let appearance = TextAppearance(
            value: value, query: query, font: font, alignment: alignment, foreground: foreground)
        guard rendered != appearance else { return }
        rendered = appearance
        if let query, !query.isEmpty, value.range(of: query, options: .caseInsensitive) != nil {
            if field.font != font { field.font = font }
            if field.alignment != alignment { field.alignment = alignment }
            let styled = highlighted(value, query: query, font: font, foreground: foreground)
            let paragraph = NSMutableParagraphStyle()
            paragraph.alignment = alignment
            paragraph.lineBreakMode = .byTruncatingTail
            styled.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: styled.length))
            field.attributedStringValue = styled
            usesAttributedText = true
        } else {
            // Reassigning plain text when leaving Find also removes old range attributes.
            if usesAttributedText || field.stringValue != value { field.stringValue = value }
            usesAttributedText = false
            if field.font != font { field.font = font }
            if field.alignment != alignment { field.alignment = alignment }
            if field.textColor != foreground { field.textColor = foreground }
        }
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        if currentMatch {
            (backgroundStyle == .emphasized ? NSColor.alternateSelectedControlTextColor : NSColor.controlAccentColor)
                .setStroke()
            let outline = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 3, yRadius: 3)
            outline.lineWidth = 2
            outline.stroke()
        }
        guard wholeCellMatch, let field = textField else { return }
        // Hug the text, like Finder and Xcode find results.
        let text = field.attributedStringValue.size()
        let width = max(0, min(text.width + 8, bounds.width - 4))
        let x = field.alignment == .right ? bounds.maxX - 6 - width : bounds.minX + 6
        let rect = NSRect(x: x, y: bounds.midY - text.height / 2 - 1, width: width, height: text.height + 2)
        NSColor.findHighlightColor.setFill()
        NSBezierPath(roundedRect: rect, xRadius: 3, yRadius: 3).fill()
    }
}

/// Text with every case-insensitive occurrence of `query` highlighted.
func highlighted(_ text: String, query: String, font: NSFont, foreground: NSColor = .labelColor)
    -> NSMutableAttributedString
{
    let result = NSMutableAttributedString(
        string: text,
        attributes: [
            .font: font, .foregroundColor: foreground,
        ])
    guard !query.isEmpty else { return result }
    var range = text.startIndex..<text.endIndex
    while let found = text.range(of: query, options: [.caseInsensitive], range: range) {
        result.addAttributes(
            [
                .backgroundColor: NSColor.findHighlightColor,
                // The highlight is light in both appearances; keep text dark on it.
                .foregroundColor: NSColor.black,
            ], range: NSRange(found, in: text))
        range = found.upperBound..<text.endIndex
    }
    return result
}
