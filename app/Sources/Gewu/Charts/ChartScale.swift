import SwiftUI

extension AxisScale {
    var title: String {
        switch self {
        case .linear: "Linear"
        case .log10: "Log10"
        }
    }
}

struct ScalePicker: View {
    let title: String
    @Binding var selection: AxisScale

    var body: some View {
        Picker(title, selection: $selection) {
            Text(AxisScale.linear.title).tag(AxisScale.linear)
            Text(AxisScale.log10.title).tag(AxisScale.log10)
        }
        .pickerStyle(.menu).controlSize(.small)
    }
}

struct HistogramOmissions: View {
    let histogram: Histogram

    var body: some View {
        if histogram.nonPositiveCount > 0 {
            Text("\(formatCount(histogram.nonPositiveCount)) values omitted: zero or negative")
                .font(.caption2).foregroundStyle(.secondary)
        }
        if histogram.infiniteCount > 0 {
            Text("\(formatCount(histogram.infiniteCount)) infinite values omitted")
                .font(.caption2).foregroundStyle(.secondary)
        }
    }
}
