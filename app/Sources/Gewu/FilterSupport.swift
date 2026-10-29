import AppKit

/// What to do with null values in a column filter.
enum NullMode: Int {
    case include, exclude, only

    init(_ filters: [Filter]) {
        self = .include
        for filter in filters {
            switch filter {
            case let .isNull(_, negate): self = negate ? .exclude : .only
            case let .range(_, _, _, includeNulls), let .contains(_, _, _, includeNulls):
                // An explicit null-only condition takes precedence if filters are combined.
                if self != .only { self = includeNulls ? .include : .exclude }
            default: break
            }
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
