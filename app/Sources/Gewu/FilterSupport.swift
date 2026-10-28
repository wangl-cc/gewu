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

}

func formatCount(_ n: UInt64) -> String {
    NumberFormatter.localizedString(from: NSNumber(value: n), number: .decimal)
}
