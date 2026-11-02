import SwiftUI

enum NullMode: Int {
    case include, exclude, only

    init(_ filter: Filter?) {
        if let filter, case .exact(let values, let exclude) = filter.condition, values.isEmpty, !exclude,
            filter.includeNulls
        {
            self = .only
        } else {
            self = filter?.includeNulls == false ? .exclude : .include
        }
    }

    func applying(to condition: FilterCondition?) -> Filter? {
        if self == .only { return Filter(condition: .exact(values: [], exclude: false), includeNulls: true) }
        guard let condition else {
            return self == .include ? nil : Filter(condition: .exact(values: [], exclude: true), includeNulls: false)
        }
        if case .exact(let values, let exclude) = condition, values.isEmpty, exclude, self == .include { return nil }
        return Filter(condition: condition, includeNulls: self == .include)
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
        }.labelsHidden().fixedSize()
    }
}
