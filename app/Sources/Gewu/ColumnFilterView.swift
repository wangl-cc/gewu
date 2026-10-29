import SwiftUI

struct ColumnFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let info: ColumnInfo
    let state: ViewState
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var spec: FilterSpec?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text(info.name).font(.headline)
                Text(shortTypeName(info.typeName)).font(.caption).foregroundStyle(.secondary)
            }
            if let spec {
                editor(spec)
            } else if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            } else {
                ProgressView("Loading filter…").frame(maxWidth: .infinity, minHeight: 120)
            }
        }
        .padding(14)
        .frame(width: 420)
        .task { await load() }
    }

    @ViewBuilder private func editor(_ spec: FilterSpec) -> some View {
        switch spec.editor {
        case let .range(_, distribution):
            RangeFilterView(doc: doc, column: column, nullable: spec.nullable,
                            distribution: distribution, current: current, onApply: onApply)
        case let .choices(values):
            ChoiceFilterView(column: column, values: values, current: current, onApply: onApply)
        case .contains:
            ContainsFilterView(column: column, nullable: spec.nullable, current: current, onApply: onApply)
        case .nullOnly:
            NullFilterView(column: column, current: current, onApply: onApply)
        }
    }

    @MainActor private func load() async {
        do {
            let doc = doc, column = column, state = state
            let spec = try await Task.detached { try doc.filterSpec(column: column, state: state) }.value
            guard !Task.isCancelled else { return }
            self.spec = spec
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }
}

private struct ChoiceFilterView: View {
    let column: UInt32
    let values: [DistinctValue]
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var query = ""
    @State private var checked: Set<UInt64> = []
    private var shown: [DistinctValue] {
        values.filter { query.isEmpty || ($0.label ?? "(null)").localizedCaseInsensitiveContains(query) }
    }
    private var mostCount: UInt64 { max(values.map(\.count).max() ?? 0, 1) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("Find value", text: $query).textFieldStyle(.roundedBorder)
            List {
                ForEach(shown, id: \.key) { value in
                    HStack {
                        Toggle(value.label ?? "(null)", isOn: Binding(
                            get: { checked.contains(value.key) },
                            set: { selected in
                                if selected { checked.insert(value.key) } else { checked.remove(value.key) }
                            }
                        ))
                        .toggleStyle(.checkbox)
                        .lineLimit(1)
                        Spacer(minLength: 6)
                        ProgressView(value: Double(value.count), total: Double(mostCount)).frame(width: 42)
                        Text(formatCount(value.count)).monospacedDigit().foregroundStyle(.secondary)
                    }
                    .help(value.label ?? "(null)")
                }
            }
            .listStyle(.bordered)
            .frame(height: 230)
            Text("\(formatCount(UInt64(values.count))) values").font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Select All") { checked.formUnion(shown.map(\.key)) }
                Button("Clear") { checked.subtract(shown.map(\.key)) }
                Spacer()
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
        }
        .onAppear {
            checked = Set(values.map(\.key))
            for filter in current {
                switch filter {
                case let .inValues(_, keys): checked.formIntersection(keys)
                case let .isNull(_, negate):
                    checked.formIntersection(values.filter { ($0.label == nil) != negate }.map(\.key))
                default: break
                }
            }
        }
    }

    private func apply() {
        let everything = Set(values.map(\.key)).isSubset(of: checked)
        onApply(everything ? [] : [.inValues(column: column, keys: checked.sorted())])
    }
}

private struct ContainsFilterView: View {
    let column: UInt32
    let nullable: Bool
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var query = ""
    @State private var caseSensitive = false
    @State private var nullMode = NullMode.include

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("Contains…", text: $query).textFieldStyle(.roundedBorder).onSubmit { apply() }
            HStack {
                if nullable { NullModePicker(selection: $nullMode) }
                Spacer()
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
        }
        .onAppear {
            nullMode = NullMode(current)
            for case let .contains(_, text, sensitive) in current {
                query = text
                caseSensitive = sensitive
            }
        }
    }

    private func apply() {
        if nullMode == .only, let filter = nullMode.filter(column: column) {
            onApply([filter])
        } else if !query.isEmpty {
            onApply([.contains(column: column, text: query, caseSensitive: caseSensitive)])
        } else {
            onApply(nullMode.filter(column: column).map { [$0] } ?? [])
        }
    }
}

private struct NullFilterView: View {
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

private struct RangeFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let nullable: Bool
    let distribution: RangeHistogram?
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
              distribution.ranges.count == distribution.histogram.counts.count else { return nil }
        return distribution
    }

    private func plotHistogram(_ distribution: RangeHistogram) -> Histogram {
        let h = distribution.histogram
        return Histogram(lo: 0, hi: Double(distribution.ranges.count), integer: false,
                         counts: h.counts, nullCount: h.nullCount, nanCount: h.nanCount, total: h.total)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let chart {
                HistogramChart(histogram: plotHistogram(chart), selection: Binding(
                    get: { selection }, set: selectBins
                ), axisLabels: (chart.minLabel, chart.maxLabel))
                    .frame(height: 140)
                    .accessibilityLabel("Distribution; edit the minimum and maximum fields to select a range")
                Text("Drag to select a range · Double-click to reset").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                TextField("Min", text: Binding(get: { minimum }, set: {
                    minimum = $0
                    lowerEdited = true
                    selectedBins = nil
                    selection.snapped = false
                    error = nil
                })).accessibilityLabel("Minimum")
                Text("–").foregroundStyle(.secondary)
                TextField("Max", text: Binding(get: { maximum }, set: {
                    maximum = $0
                    upperEdited = true
                    selectedBins = nil
                    selection.snapped = false
                    error = nil
                })).accessibilityLabel("Maximum")
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
    }

    private func restore() {
        reset()
        nullMode = NullMode(current)
        for case let .range(_, min, max) in current {
            lowerBound = min
            upperBound = max
        }
        minimum = lowerBound?.display() ?? ""
        maximum = upperBound?.display() ?? ""
        if let chart {
            let count = chart.ranges.count
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
              (hi - lo).isFinite else { return upper ? count : 0 }
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
        if let selectedBins {
            let count = histogram.counts[selectedBins].reduce(0, +)
            if selectedBins.count == histogram.counts.count { return "All \(formatCount(base)) values" }
            return "\(formatCount(count)) of \(formatCount(base)) values selected"
        }
        let count = histogram.counts.enumerated().reduce(0.0) { total, pair in
            let start = Double(pair.offset)
            let overlap = Swift.max(0, Swift.min(start + 1, selection.upper) - Swift.max(start, selection.lower))
            return total + Double(pair.element) * overlap
        }
        return "\(selection.snapped ? "" : "≈ ")\(formatCount(UInt64(count.rounded()))) of \(formatCount(base)) values selected"
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
                let filter = Filter.range(column: column, min: min, max: max)
                try doc.validateFilter(filter: filter)
                onApply([filter])
            }
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func parse(_ text: String) throws -> Bound? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : try doc.parseBound(column: column, text: text)
    }
}

private struct NullModePicker: View {
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
