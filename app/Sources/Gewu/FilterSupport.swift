import AppKit

/// What to do with null values in a column filter.
enum NullMode: Int {
    case include, exclude, only

    init(_ filters: [Filter]) {
        self = .include
        for case let .isNull(_, negate) in filters {
            self = negate ? .exclude : .only
        }
    }

    func filter(column: UInt32) -> Filter? {
        switch self {
        case .include: nil
        case .exclude: .isNull(column: column, negate: true)
        case .only: .isNull(column: column, negate: false)
        }
    }

    /// A pop-up button for choosing the mode.
    static func popUp(nullCount: UInt64?, target: AnyObject, action: Selector) -> NSPopUpButton {
        let button = NSPopUpButton(frame: .zero, pullsDown: false)
        let suffix = nullCount.map { " (\(formatCount($0)))" } ?? ""
        button.addItems(withTitles: ["Include nulls", "Hide nulls", "Only nulls\(suffix)"])
        button.controlSize = .small
        button.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        button.target = target
        button.action = action
        return button
    }
}

func formatCount(_ n: UInt64) -> String {
    NumberFormatter.localizedString(from: NSNumber(value: n), number: .decimal)
}

/// Small secondary-colored label.
func noteLabel(_ text: String = "") -> NSTextField {
    let label = NSTextField(labelWithString: text)
    label.textColor = .secondaryLabelColor
    label.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
    label.lineBreakMode = .byTruncatingTail
    return label
}
