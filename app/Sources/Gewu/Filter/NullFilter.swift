import SwiftUI

/// What to do with null values in a column filter.
enum NullMode: Int {
    case include, exclude, only

    init(_ filters: [Filter]) {
        self = .include
        for filter in filters {
            switch filter {
            case .isNull(_, let negate): self = negate ? .exclude : .only
            case .range(_, _, _, let includeNulls), .contains(_, _, _, let includeNulls):
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

struct NullFilterView: View {
    let column: UInt32
    let current: [Filter]
    let onApply: ([Filter]) -> Void
    @State private var nullMode = NullMode.include

    var body: some View {
        HStack {
            NullModePicker(selection: $nullMode)
            Spacer()
            Button("Apply") { onApply(nullMode.filter(column: column).map { [$0] } ?? []) }
                .keyboardShortcut(.defaultAction)
        }
        .onAppear { nullMode = NullMode(current) }
    }
}

struct NullModePicker: View {
    @Binding var selection: NullMode
    var nullCount: UInt64?

    var body: some View {
        Picker("Null values", selection: $selection) {
            Text("Include nulls").tag(NullMode.include)
            Text("Hide nulls").tag(NullMode.exclude)
            Text("Only nulls" + (nullCount.map { " (\(formatCount($0)))" } ?? ""))
                .tag(NullMode.only)
        }
        .labelsHidden()
        .fixedSize()
    }
}
