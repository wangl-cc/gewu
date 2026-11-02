import SwiftUI

struct RangeDraft {
    var minimum = ""
    var maximum = ""
    var lowerBound: RangeBound?
    var upperBound: RangeBound?
    var lowerEdited = false
    var upperEdited = false
    init(_ filter: Filter?) {
        if let filter, case .range(let min, let max) = filter.condition {
            minimum = min ?? ""
            maximum = max ?? ""
            lowerEdited = true
            upperEdited = true
        }
    }
    mutating func restoreLabels(from histogram: RangeHistogram?) {
        guard let histogram else { return }
        if lowerEdited, let bound = histogram.ranges.compactMap(\.min).first(where: { $0.value == minimum }) {
            lowerBound = bound
            minimum = bound.label
            lowerEdited = false
        }
        if upperEdited, let bound = histogram.ranges.compactMap(\.max).first(where: { $0.value == maximum }) {
            upperBound = bound
            maximum = bound.label
            upperEdited = false
        }
    }
    var condition: FilterCondition? {
        let min = lowerEdited ? minimum.nilIfBlank : lowerBound?.value
        let max = upperEdited ? maximum.nilIfBlank : upperBound?.value
        return min == nil && max == nil ? nil : .range(min: min, max: max)
    }
}

extension String {
    fileprivate var nilIfBlank: String? {
        let text = trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}

struct RangeFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let distribution: RangeHistogram?
    @Binding var bins: UInt32
    @Binding var scale: AxisScale
    let supportsLog: Bool
    @Binding var draft: RangeDraft

    @State private var selection = HistogramSelection(lower: 0, upper: 1)
    @State private var selectedBins: Range<Int>?
    @State private var frequencyScale: AxisScale = .linear

    // Geometry uses ordinal bin edges, so large adjacent values never collapse.
    private var chart: RangeHistogram? {
        guard let distribution, !distribution.ranges.isEmpty,
            distribution.ranges.count == distribution.histogram.counts.count
        else { return nil }
        return distribution
    }

    private func plotHistogram(_ distribution: RangeHistogram) -> Histogram {
        var h = distribution.histogram
        h.lo = 0
        h.hi = Double(distribution.ranges.count)
        h.integer = false
        h.binEdges = []
        return h
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if supportsLog {
                ScalePicker(title: "Binning", selection: $scale)
                    .help("Log10 bins include only finite positive values; typed bounds still use original values.")
            }
            if let chart {
                ScalePicker(title: "Frequency", selection: $frequencyScale)
                Picker("Maximum bins", selection: $bins) {
                    ForEach([10, 20, 40, 60] as [UInt32], id: \.self) { count in
                        Text(count.formatted()).tag(count)
                    }
                }
                .controlSize(.small)
                HistogramChart(
                    histogram: plotHistogram(chart),
                    selection: Binding(
                        get: { selection }, set: selectBins
                    ), frequencyScale: frequencyScale
                )
                .frame(height: 140)
                .accessibilityLabel("Distribution; edit the draft.minimum and draft.maximum fields to select a range")
                Text("Drag to select a range · Double-click to select all plotted values")
                    .font(.caption).foregroundStyle(.secondary)
            } else if distribution?.histogram.scale == .log10 {
                Text("No positive finite values to plot").font(.caption).foregroundStyle(.secondary)
            }
            if let distribution { HistogramOmissions(histogram: distribution.histogram) }
            HStack {
                TextField(
                    "Min",
                    text: Binding(
                        get: { draft.minimum },
                        set: {
                            draft.minimum = $0
                            draft.lowerEdited = true
                            selectedBins = nil
                            selection.snapped = false
                        })
                ).accessibilityLabel("Minimum")
                Text("–").foregroundStyle(.secondary)
                TextField(
                    "Max",
                    text: Binding(
                        get: { draft.maximum },
                        set: {
                            draft.maximum = $0
                            draft.upperEdited = true
                            selectedBins = nil
                            selection.snapped = false
                        })
                ).accessibilityLabel("Maximum")
            }
            .textFieldStyle(.roundedBorder)
            .monospacedDigit()
            if let chart, !draft.lowerEdited, !draft.upperEdited {
                Text(selectionSummary(chart.histogram)).font(.caption).foregroundStyle(.secondary)
            }
            Button("Reset", action: reset)
        }
        .onAppear { projectSelection() }
        .onChange(of: distribution?.histogram) { _, _ in
            projectSelection()
        }
    }

    // Rebinning changes only the chart geometry, never the exact drafted bounds.
    private func projectSelection() {
        selectedBins = nil
        selection.snapped = false
        if let chart {
            let count = chart.ranges.count
            if draft.lowerEdited || draft.upperEdited {
                selection = HistogramSelection(lower: 0, upper: Double(count), snapped: false)
                return
            }
            let first: Int?
            if let lowerBound = draft.lowerBound {
                first = chart.ranges.firstIndex { $0.min?.value == lowerBound.value }
            } else {
                first = 0
            }
            let end: Int?
            if let upperBound = draft.upperBound {
                end = chart.ranges.firstIndex { $0.max?.value == upperBound.value }.map { $0 + 1 }
            } else {
                end = count
            }
            selection.lower = first.map(Double.init) ?? approximateEdge(draft.lowerBound, upper: false, chart: chart)
            selection.upper = end.map(Double.init) ?? approximateEdge(draft.upperBound, upper: true, chart: chart)
            selection.upper = Swift.max(selection.lower, selection.upper)
            if let first, let end, first < end {
                selectedBins = first..<end
                selection.snapped = true
            } else {
                selectedBins = nil
                selection.snapped = false
            }
        }
    }

    /// Only a visual approximation for manually typed endpoints which are not bin edges.
    private func approximateEdge(_ bound: RangeBound?, upper: Bool, chart: RangeHistogram) -> Double {
        let count = Double(chart.ranges.count)
        guard let bound else { return upper ? count : 0 }
        let h = chart.histogram
        guard let lo = h.lo, let hi = h.hi, hi > lo, lo.isFinite, hi.isFinite,
            (hi - lo).isFinite
        else { return upper ? count : 0 }
        let value: Double
        if h.scale == .log10 {
            let original = bound.coordinate
            guard original > 0 else { return 0 }
            value = log10(original)
        } else {
            value = bound.coordinate + (upper && h.integer ? 1 : 0)
        }
        if !h.binEdges.isEmpty {
            for index in 0..<chart.ranges.count where value < h.binEdges[index + 1] {
                let start = h.binEdges[index]
                let fraction = Swift.max(0, (value - start) / (h.binEdges[index + 1] - start))
                return Double(index) + fraction
            }
            return count
        }
        let edge = (value - lo) / (hi - lo) * count
        guard !edge.isNaN else { return upper ? count : 0 }
        return Swift.min(count, Swift.max(0, edge))
    }

    private func selectBins(_ proposed: HistogramSelection) {
        guard let chart else { return }
        let count = chart.ranges.count
        let first = Swift.min(count - 1, Swift.max(0, Int(proposed.lower.rounded())))
        let end = Swift.min(count, Swift.max(first + 1, Int(proposed.upper.rounded())))
        // Use the core's exact endpoint objects; chart coordinates are only bin selectors.
        draft.lowerBound = chart.ranges[first].min
        draft.upperBound = chart.ranges[end - 1].max
        draft.minimum = draft.lowerBound?.label ?? ""
        draft.maximum = draft.upperBound?.label ?? ""
        draft.lowerEdited = false
        draft.upperEdited = false
        selectedBins = first..<end
        selection = HistogramSelection(lower: Double(first), upper: Double(end))
    }

    private func reset() {
        draft.lowerBound = nil
        draft.upperBound = nil
        draft.minimum = ""
        draft.maximum = ""
        draft.lowerEdited = false
        draft.upperEdited = false
        selection = HistogramSelection(lower: 0, upper: Double(chart?.ranges.count ?? 1))
        selectedBins = chart.map { 0..<$0.ranges.count }
    }

    private func selectionSummary(_ histogram: Histogram) -> String {
        guard !histogram.counts.isEmpty else { return "" }
        let base = histogram.counts.reduce(0, +)
        if let selectedBins, selectedBins.lowerBound >= 0, selectedBins.upperBound <= histogram.counts.count {
            let count = histogram.counts[selectedBins].reduce(0, +)
            if selectedBins.count == histogram.counts.count { return "All \(formatCount(base)) plotted values" }
            return "\(formatCount(count)) of \(formatCount(base)) values selected"
        }
        let count = histogram.counts.enumerated().reduce(0.0) { total, pair in
            let start = Double(pair.offset)
            let overlap = Swift.max(0, Swift.min(start + 1, selection.upper) - Swift.max(start, selection.lower))
            return total + Double(pair.element) * overlap
        }
        return
            "\(selection.snapped ? "" : "≈ ")\(formatCount(UInt64(count.rounded()))) of \(formatCount(base)) values selected"
    }

}
