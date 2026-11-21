import GewuKit
import SwiftUI

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
