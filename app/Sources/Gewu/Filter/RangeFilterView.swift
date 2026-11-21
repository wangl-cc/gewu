import GewuKit
import SwiftUI

struct RangeFilterView: View {
    let distribution: RangeHistogram?
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
            HStack(alignment: .bottom, spacing: 12) {
                VStack(alignment: .leading, spacing: 5) {
                    Text("Minimum").font(.caption).foregroundStyle(.secondary)
                    TextField(
                        "No minimum",
                        text: Binding(
                            get: { draft.minimum },
                            set: {
                                draft.typeMinimum($0)
                                selectedBins = nil
                                selection.snapped = false
                            })
                    ).accessibilityLabel("Minimum")
                }
                VStack(alignment: .leading, spacing: 5) {
                    Text("Maximum").font(.caption).foregroundStyle(.secondary)
                    TextField(
                        "No maximum",
                        text: Binding(
                            get: { draft.maximum },
                            set: {
                                draft.typeMaximum($0)
                                selectedBins = nil
                                selection.snapped = false
                            })
                    ).accessibilityLabel("Maximum")
                }
            }
            .textFieldStyle(.roundedBorder)
            .monospacedDigit()
            HStack {
                Text(rangeSummary).font(.caption).foregroundStyle(.secondary)
                Spacer(minLength: 4)
                if scale == .log10 { Text("Log bins").font(.caption2).foregroundStyle(.secondary) }
                if frequencyScale == .log10 { Text("Log frequency").font(.caption2).foregroundStyle(.secondary) }
                Menu {
                    if supportsLog {
                        Picker("Bin spacing", selection: $scale) {
                            Text("Linear").tag(AxisScale.linear)
                            Text("Logarithmic").tag(AxisScale.log10)
                        }
                    }
                    Picker("Frequency", selection: $frequencyScale) {
                        Text("Linear").tag(AxisScale.linear)
                        Text("Logarithmic").tag(AxisScale.log10)
                    }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton).fixedSize().accessibilityLabel("Chart display options")
            }
            if let chart {
                HistogramChart(
                    histogram: plotHistogram(chart),
                    selection: Binding(get: { selection }, set: selectBins), frequencyScale: frequencyScale
                )
                .frame(height: 145)
                .accessibilityLabel("Distribution; use Minimum and Maximum to enter exact bounds")
                .help("Drag to select a range. Double-click to select all plotted values.")
            } else {
                Text(
                    distribution == nil
                        ? "Loading distribution…"
                        : (distribution?.histogram.scale == .log10
                            ? "No positive finite values to plot" : "No finite values to plot")
                )
                .font(.caption).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 145)
            }
            if let distribution { HistogramOmissions(histogram: distribution.histogram) }
        }
        .onAppear { projectSelection() }
        .onChange(of: distribution?.histogram) { _, _ in
            projectSelection()
        }
        .onChange(of: draft.condition) { _, _ in projectSelection() }
    }

    private var rangeSummary: String {
        guard let chart else { return "" }
        if draft.isTyped { return "Distribution" }
        return selectionSummary(chart.histogram)
    }

    // Rebinning changes only the chart geometry, never the exact drafted bounds.
    private func projectSelection() {
        selectedBins = nil
        selection.snapped = false
        if let chart {
            let count = chart.ranges.count
            if draft.isTyped {
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
        draft.select(first..<end, of: chart)
        selectedBins = first..<end
        selection = HistogramSelection(lower: Double(first), upper: Double(end))
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
