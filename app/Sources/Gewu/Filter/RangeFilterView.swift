import SwiftUI

struct RangeFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let nullable: Bool
    let distribution: RangeHistogram?
    @Binding var bins: UInt32
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var selection = HistogramSelection(lower: 0, upper: 1)
    @State private var selectedBins: Range<Int>?
    @State private var minimum = ""
    @State private var maximum = ""
    @State private var lowerBound: Bound?
    @State private var upperBound: Bound?
    @State private var lowerEdited = false
    @State private var upperEdited = false
    @State private var nullMode = NullMode.include
    @State private var error: String?

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
        return h
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let chart {
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
                    )
                )
                .frame(height: 140)
                .accessibilityLabel("Distribution; edit the minimum and maximum fields to select a range")
                Text("Drag to select a range · Double-click to reset").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                TextField(
                    "Min",
                    text: Binding(
                        get: { minimum },
                        set: {
                            minimum = $0
                            lowerEdited = true
                            selectedBins = nil
                            selection.snapped = false
                            error = nil
                        })
                ).accessibilityLabel("Minimum")
                Text("–").foregroundStyle(.secondary)
                TextField(
                    "Max",
                    text: Binding(
                        get: { maximum },
                        set: {
                            maximum = $0
                            upperEdited = true
                            selectedBins = nil
                            selection.snapped = false
                            error = nil
                        })
                ).accessibilityLabel("Maximum")
            }
            .textFieldStyle(.roundedBorder)
            .monospacedDigit()
            .onSubmit { apply() }
            if let chart, !lowerEdited, !upperEdited {
                Text(selectionSummary(chart.histogram)).font(.caption).foregroundStyle(.secondary)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                if nullable { NullModePicker(selection: $nullMode, nullCount: distribution?.histogram.nullCount) }
                Spacer()
                Button("Reset", action: reset)
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
        }
        .onAppear { restore() }
        .onChange(of: distribution?.histogram.counts.count) { _, _ in
            projectSelection()
        }
    }

    private func restore() {
        reset()
        nullMode = NullMode(current)
        for case .range(_, let min, let max, _) in current {
            lowerBound = min
            upperBound = max
        }
        minimum = lowerBound?.display() ?? ""
        maximum = upperBound?.display() ?? ""
        projectSelection()
    }

    // Rebinning changes only the chart geometry, never the exact drafted bounds.
    private func projectSelection() {
        selectedBins = nil
        selection.snapped = false
        if let chart {
            let count = chart.ranges.count
            if lowerEdited || upperEdited {
                selection = HistogramSelection(lower: 0, upper: Double(count), snapped: false)
                return
            }
            let first: Int?
            if let lowerBound {
                first = chart.ranges.firstIndex { $0.min?.sameValue(other: lowerBound) == true }
            } else {
                first = 0
            }
            let end: Int?
            if let upperBound {
                end = chart.ranges.firstIndex { $0.max?.sameValue(other: upperBound) == true }.map { $0 + 1 }
            } else {
                end = count
            }
            selection.lower = first.map(Double.init) ?? approximateEdge(lowerBound, upper: false, chart: chart)
            selection.upper = end.map(Double.init) ?? approximateEdge(upperBound, upper: true, chart: chart)
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
    private func approximateEdge(_ bound: Bound?, upper: Bool, chart: RangeHistogram) -> Double {
        let count = Double(chart.ranges.count)
        guard let bound else { return upper ? count : 0 }
        let h = chart.histogram
        guard let lo = h.lo, let hi = h.hi, hi > lo, lo.isFinite, hi.isFinite,
            (hi - lo).isFinite
        else { return upper ? count : 0 }
        let value = bound.axisValue() + (upper && h.integer ? 1 : 0)
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
        lowerBound = chart.ranges[first].min
        upperBound = chart.ranges[end - 1].max
        minimum = lowerBound?.display() ?? ""
        maximum = upperBound?.display() ?? ""
        lowerEdited = false
        upperEdited = false
        selectedBins = first..<end
        selection = HistogramSelection(lower: Double(first), upper: Double(end))
        error = nil
    }

    private func reset() {
        lowerBound = nil
        upperBound = nil
        minimum = ""
        maximum = ""
        lowerEdited = false
        upperEdited = false
        nullMode = .include
        error = nil
        selection = HistogramSelection(lower: 0, upper: Double(chart?.ranges.count ?? 1))
        selectedBins = chart.map { 0..<$0.ranges.count }
    }

    private func selectionSummary(_ histogram: Histogram) -> String {
        guard !histogram.counts.isEmpty else { return "" }
        let base = histogram.total - histogram.nullCount - histogram.nanCount
        if let selectedBins, selectedBins.lowerBound >= 0, selectedBins.upperBound <= histogram.counts.count {
            let count = histogram.counts[selectedBins].reduce(0, +)
            if selectedBins.count == histogram.counts.count { return "All \(formatCount(base)) values" }
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

    private func apply() {
        if nullMode == .only, let filter = nullMode.filter(column: column) {
            onApply([filter])
            return
        }
        do {
            let min = lowerEdited ? try parse(minimum) : lowerBound
            let max = upperEdited ? try parse(maximum) : upperBound
            if min == nil && max == nil {
                onApply(nullMode.filter(column: column).map { [$0] } ?? [])
            } else {
                let filter = Filter.range(
                    column: column, min: min, max: max,
                    includeNulls: nullMode == .include)
                try doc.validateFilter(filter: filter)
                onApply([filter])
            }
        } catch {
            self.error = error.userMessage
        }
    }

    private func parse(_ text: String) throws -> Bound? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : try doc.parseBound(column: column, text: text)
    }
}
