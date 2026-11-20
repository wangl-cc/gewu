import GewuKit
import SwiftUI

enum NullMode: Int {
    case include, exclude, only

    init(_ filter: Filter?) {
        switch filter {
        case .onlyNulls: self = .only
        case .notNull: self = .exclude
        case .condition(_, let includeNulls): self = includeNulls ? .include : .exclude
        case nil: self = .include
        }
    }

    /// The filter for `condition` under this mode. Excluding no values is no condition.
    func applying(to condition: FilterCondition?) -> Filter? {
        var condition = condition
        if case .exact(let values, true) = condition, values.isEmpty { condition = nil }
        switch (self, condition) {
        case (.only, _): return .onlyNulls
        case (.exclude, nil): return .notNull
        case (.include, nil): return nil
        case (_, let condition?): return .condition(condition, includeNulls: self == .include)
        }
    }
}

struct NullModePicker: View {
    @Binding var selection: NullMode
    var nullCount: UInt64?
    var body: some View {
        Picker("Null values", selection: $selection) {
            Text("Include nulls").tag(NullMode.include)
            Text("Hide nulls").tag(NullMode.exclude)
            Text("Only nulls" + (nullCount.map { " (\(formatCount($0)))" } ?? "")).tag(NullMode.only)
        }.labelsHidden().fixedSize().controlSize(.small)
    }
}
