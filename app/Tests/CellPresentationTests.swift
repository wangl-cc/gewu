import AppKit
import Testing

@Suite @MainActor
struct CellPresentationTests {
    @Test func unchangedContentAndCurrentMatchDoNotRewriteText() {
        let cell = MatchCellView()
        let field = CountingTextField(labelWithString: "")
        cell.addSubview(field)
        cell.textField = field
        let font = NSFont.systemFont(ofSize: 12)
        cell.configure(text: "a value", font: font, alignment: .left)
        let plainWrites = field.plainWrites
        let attributedWrites = field.attributedWrites
        let styleWrites = field.styleWrites

        cell.configure(text: "a value", font: font, alignment: .left)
        cell.updateFind(query: nil, wholeMatch: false, current: true)
        cell.refreshAppearance()
        cell.updateStyle(font: font, alignment: .left)

        #expect(field.plainWrites == plainWrites)
        #expect(field.attributedWrites == attributedWrites)
        #expect(field.styleWrites == styleWrites)
        #expect(cell.currentMatch)
        #expect(field.stringValue == "a value")
    }

    @Test func findDecorationUsesRetainedContentAndReturnsToPlainText() {
        let cell = MatchCellView()
        let field = CountingTextField(labelWithString: "")
        cell.addSubview(field)
        cell.textField = field
        cell.configure(text: "alpha beta", font: .systemFont(ofSize: 12), alignment: .left)
        let plainAttributedWrites = field.attributedWrites
        cell.updateFind(query: "missing", wholeMatch: false, current: false)
        #expect(field.attributedWrites == plainAttributedWrites, "No actual substring match needs rich text")

        cell.updateFind(query: "beta", wholeMatch: false, current: true)
        #expect(field.stringValue == "alpha beta")
        #expect(
            field.attributedStringValue.attribute(.backgroundColor, at: 6, effectiveRange: nil) as? NSColor
                == .findHighlightColor)
        let highlightedWrites = field.attributedWrites
        cell.updateFind(query: "beta", wholeMatch: false, current: false)
        #expect(field.attributedWrites == highlightedWrites, "Moving the current outline must not rebuild rich text")

        cell.updateFind(query: nil, wholeMatch: false, current: false)
        #expect(field.stringValue == "alpha beta")
        #expect(field.attributedStringValue.attribute(.backgroundColor, at: 6, effectiveRange: nil) == nil)
        #expect(field.textColor == .labelColor)
        #expect(field.attributedWrites == highlightedWrites)
    }

    @Test func selectionAndNullStylesSurvivePlainAndHighlightedReuse() {
        let cell = MatchCellView()
        let field = NSTextField(labelWithString: "")
        cell.addSubview(field)
        cell.textField = field
        let fonts = CellFonts()
        cell.configure(text: "null", font: fonts.null, alignment: .right, muted: true)
        #expect(field.font == fonts.null && field.alignment == .right)
        #expect(field.textColor == .tertiaryLabelColor)
        cell.backgroundStyle = .emphasized
        #expect(field.textColor == .alternateSelectedControlTextColor)
        cell.backgroundStyle = .normal
        #expect(field.textColor == .tertiaryLabelColor)

        cell.configure(text: "12", font: fonts.digits, alignment: .right, wholeMatch: true)
        #expect(field.textColor == .black && cell.wholeCellMatch)
        cell.updateFind(query: nil, wholeMatch: false, current: false)
        #expect(field.textColor == .labelColor && !cell.muted)
        cell.updateStyle(font: .systemFont(ofSize: 16), alignment: .left)
        #expect(field.font?.pointSize == 16 && field.alignment == .left)
        #expect(field.stringValue == "12")
    }
}

@MainActor
private final class CountingTextField: NSTextField {
    private(set) var plainWrites = 0
    private(set) var attributedWrites = 0
    private(set) var styleWrites = 0

    override var stringValue: String {
        get { super.stringValue }
        set {
            plainWrites += 1
            super.stringValue = newValue
        }
    }

    override var attributedStringValue: NSAttributedString {
        get { super.attributedStringValue }
        set {
            attributedWrites += 1
            super.attributedStringValue = newValue
        }
    }

    override var font: NSFont? {
        get { super.font }
        set {
            styleWrites += 1
            super.font = newValue
        }
    }

    override var alignment: NSTextAlignment {
        get { super.alignment }
        set {
            styleWrites += 1
            super.alignment = newValue
        }
    }

    override var textColor: NSColor? {
        get { super.textColor }
        set {
            styleWrites += 1
            super.textColor = newValue
        }
    }
}
