import SwiftUI

struct ColumnFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let info: ColumnInfo
    let kind: ColumnKind
    let state: ViewState
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text(info.name).font(.headline)
                Text(shortTypeName(info.typeName)).font(.caption).foregroundStyle(.secondary)
            }
            if kind != .text, !info.typeName.hasPrefix("Interval") {
                RangeFilterView(doc: doc, column: column, temporal: kind == .temporal,
                                state: state, current: current, onApply: onApply)
            } else {
                ValueFilterView(doc: doc, column: column, state: state,
                                current: current, onApply: onApply)
            }
        }
        .padding(14)
        .frame(width: 420)
    }
}

private struct ValueFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let state: ViewState
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var result: DistinctResult?
    @State private var query = ""
    @State private var checked: Set<UInt64> = []
    @State private var nullMode = NullMode.include
    @State private var error: String?
    @State private var loading = true
    @State private var mostCount: UInt64 = 1
    @State private var values: [DistinctValue] = []

    private var containsMode: Bool { (result?.total ?? 0) > 500 }
    private var shown: [DistinctValue] {
        values.filter { query.isEmpty || ($0.label ?? "(null)").localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField(containsMode ? "Contains…" : "Find value", text: $query)
                .textFieldStyle(.roundedBorder)
                .disabled(loading)
            if loading {
                ProgressView("Loading values…").frame(maxWidth: .infinity, minHeight: 120)
            } else if let error {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            } else {
                List {
                    ForEach(shown, id: \.key) { value in
                        HStack {
                            if containsMode {
                                Button(value.label ?? "(null)") { query = value.label ?? "" }
                                    .buttonStyle(.plain)
                                    .lineLimit(1)
                            } else {
                                Toggle(value.label ?? "(null)", isOn: Binding(
                                    get: { checked.contains(value.key) },
                                    set: { selected in
                                        if selected { checked.insert(value.key) } else { checked.remove(value.key) }
                                    }
                                ))
                                .toggleStyle(.checkbox)
                                .lineLimit(1)
                            }
                            Spacer(minLength: 6)
                            ProgressView(value: Double(value.count), total: Double(mostCount))
                                .frame(width: 42)
                            Text(formatCount(value.count)).monospacedDigit().foregroundStyle(.secondary)
                        }
                        .help(value.label ?? "(null)")
                    }
                }
                .listStyle(.bordered)
                .frame(height: 230)
                Text(note).font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                if containsMode {
                    NullModePicker(selection: $nullMode)
                } else {
                    Button("Select All") { checked.formUnion(shown.map(\.key)) }
                    Button("Clear") { checked.subtract(shown.map(\.key)) }
                }
                Spacer()
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
            .disabled(loading || result == nil)
        }
        .task { await load() }
    }

    private var note: String {
        guard let result else { return "" }
        if !containsMode { return "\(formatCount(UInt64(values.count))) values" }
        if query.isEmpty { return "\(formatCount(result.total)) distinct values · type to filter by text" }
        let scope = result.truncated ? "first \(formatCount(UInt64(values.count))) values" : "all values"
        return "\(formatCount(UInt64(shown.count))) preview matches · searched \(scope)"
    }

    @MainActor private func load() async {
        nullMode = NullMode(current)
        do {
            let doc = doc, column = column, state = state
            let result = try await Task.detached {
                try doc.distinctValues(column: column, state: state, limit: 10_000)
            }.value
            guard !Task.isCancelled else { return }
            self.result = result
            values = result.total > 500 ? result.values.sorted { $0.count > $1.count } : result.values
            mostCount = max(result.values.map(\.count).max() ?? 1, 1)
            checked = Set(result.values.map(\.key))
            for filter in current {
                switch filter {
                case let .inValues(_, keys): checked = Set(keys)
                case let .contains(_, text, _): query = text
                default: break
                }
            }
            loading = false
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
            loading = false
        }
    }

    private func apply() {
        guard let result else { return }
        if !containsMode {
            let everything = !result.truncated && Set(values.map(\.key)).isSubset(of: checked)
            onApply(everything ? [] : [.inValues(column: column, keys: checked.sorted())])
            return
        }
        if nullMode == .only, let filter = nullMode.filter(column: column) {
            onApply([filter])
        } else if !query.isEmpty {
            onApply([.contains(column: column, text: query, caseSensitive: false)])
        } else {
            onApply(nullMode.filter(column: column).map { [$0] } ?? [])
        }
    }
}

private struct RangeFilterView: View {
    let doc: GewuDocument
    let column: UInt32
    let temporal: Bool
    let state: ViewState
    let current: [Filter]
    let onApply: ([Filter]) -> Void

    @State private var histogram: Histogram?
    @State private var selection = HistogramSelection(lower: 0, upper: 1)
    @State private var minimum = ""
    @State private var maximum = ""
    @State private var nullMode = NullMode.include
    @State private var error: String?
    @State private var loading = true
    // Typed bounds retain their original text and precision. The chart is
    // an approximate Double projection and must never rewrite a typed ID.
    @State private var lowerIsOpen = true
    @State private var upperIsOpen = true
    private enum Field: Hashable { case minimum, maximum }
    @FocusState private var focusedField: Field?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if loading {
                ProgressView("Loading distribution…").frame(maxWidth: .infinity, minHeight: 140)
            } else if let h = histogram, h.lo != nil, h.hi != nil {
                HistogramChart(histogram: h, selection: Binding(
                    get: { selection },
                    set: { selection = $0; updateFields() }
                ), labelFormatter: format)
                    .frame(height: 140)
                    .accessibilityLabel("Distribution; edit the minimum and maximum fields to select a range")
                Text("Drag to select a range · Double-click to reset")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    TextField("Min", text: Binding(get: { minimum }, set: {
                        minimum = $0
                        lowerIsOpen = $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }))
                        .accessibilityLabel("Minimum")
                        .focused($focusedField, equals: .minimum)
                    Text("–").foregroundStyle(.secondary)
                    TextField("Max", text: Binding(get: { maximum }, set: {
                        maximum = $0
                        upperIsOpen = $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }))
                        .accessibilityLabel("Maximum")
                        .focused($focusedField, equals: .maximum)
                }
                .textFieldStyle(.roundedBorder)
                .monospacedDigit()
                .onSubmit { apply() }
                Text(selectionSummary(h)).font(.caption).foregroundStyle(.secondary)
            } else if histogram != nil {
                Text("No values to plot").foregroundStyle(.secondary)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                NullModePicker(selection: $nullMode, nullCount: histogram?.nullCount)
                Spacer()
                Button("Reset", action: reset)
                Button("Apply", action: apply).keyboardShortcut(.defaultAction)
            }
            .disabled(loading || histogram == nil)
        }
        .task { await load() }
        .onChange(of: focusedField) { old, _ in
            if let old { commitField(old) }
        }
    }

    @MainActor private func load() async {
        nullMode = NullMode(current)
        do {
            let doc = doc, column = column, state = state
            let h = try await Task.detached { try doc.histogram(column: column, state: state, bins: 60) }.value
            guard !Task.isCancelled else { return }
            histogram = h
            if let lo = h.lo, let hi = h.hi {
                var selected = HistogramSelection(lower: lo, upper: hi)
                for case let .range(_, min, max) in current {
                    if let min { selected.lower = Swift.max(lo, value(min)) }
                    if let max { selected.upper = Swift.min(hi, value(max) + (h.integer ? 1 : 0)) }
                    selected.snapped = min == nil && max == nil
                }
                selected.upper = Swift.max(selected.lower, selected.upper)
                selection = selected
                updateFields()
                for case let .range(_, min, max) in current {
                    lowerIsOpen = min == nil
                    upperIsOpen = max == nil
                    if let min { minimum = formatBound(min) }
                    if let max { maximum = formatBound(max) }
                }
            }
            loading = false
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
            loading = false
        }
    }

    private func value(_ bound: Bound) -> Double {
        switch bound {
        case let .int(value): Double(value)
        case let .uInt(value): Double(value)
        case let .float(value): value
        }
    }

    private func format(_ value: Double) -> String {
        if let integer = Int64(exactly: value) {
            if temporal, let text = try? doc.formatValue(column: column, value: integer) { return text }
            if histogram?.integer == true { return String(integer) }
        }
        return String(value)
    }

    private func updateFields() {
        minimum = format(histogram?.integer == true ? selection.lower.rounded(.up) : selection.lower)
        maximum = format(histogram?.integer == true ? selection.upper.rounded(.up) - 1 : selection.upper)
        lowerIsOpen = histogram?.lo.map { selection.lower <= $0 } ?? true
        upperIsOpen = histogram?.hi.map { selection.upper >= $0 } ?? true
        error = nil
    }

    private func formatBound(_ bound: Bound) -> String {
        if temporal, case let .int(value) = bound,
           let text = try? doc.formatValue(column: column, value: value) { return text }
        switch bound {
        case let .int(value): return String(value)
        case let .uInt(value): return String(value)
        case let .float(value): return String(value)
        }
    }

    private func reset() {
        guard let lo = histogram?.lo, let hi = histogram?.hi else { return }
        selection = .init(lower: lo, upper: hi)
        nullMode = .include
        updateFields()
    }

    private func commitField(_ field: Field) {
        guard let h = histogram, let lo = h.lo, let hi = h.hi else { return }
        do {
            var next = selection
            if field == .minimum {
                next.lower = try parse(minimum).map { Swift.min(hi, Swift.max(lo, value($0))) } ?? lo
            } else {
                next.upper = try parse(maximum).map {
                    Swift.min(hi, Swift.max(lo, h.integer ? value($0).rounded(.down) + 1 : value($0)))
                } ?? hi
            }
            next.upper = Swift.max(next.lower, next.upper)
            next.snapped = false
            selection = next
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func selectionSummary(_ h: Histogram) -> String {
        guard let lo = h.lo, let hi = h.hi, hi > lo, !h.counts.isEmpty else { return "" }
        let base = h.total - h.nullCount - h.nanCount
        if selection.lower <= lo, selection.upper >= hi { return "All \(formatCount(base)) values" }
        let width = (hi - lo) / Double(h.counts.count)
        let count = h.counts.enumerated().reduce(0.0) { total, pair in
            let start = lo + Double(pair.offset) * width
            let overlap = Swift.max(0, Swift.min(start + width, selection.upper) - Swift.max(start, selection.lower))
            return total + Double(pair.element) * overlap / width
        }
        let prefix = selection.snapped ? "" : "≈ "
        return "\(prefix)\(formatCount(UInt64(count.rounded()))) of \(formatCount(base)) values selected"
    }

    private func apply() {
        if nullMode == .only, let filter = nullMode.filter(column: column) {
            onApply([filter])
            return
        }
        guard let h = histogram, h.lo != nil, h.hi != nil else {
            onApply(nullMode.filter(column: column).map { [$0] } ?? [])
            return
        }
        do {
            let min = lowerIsOpen ? nil : try parse(minimum)
            let max = upperIsOpen ? nil : try parse(maximum)
            if let min, let max, value(min) > value(max) {
                error = "Minimum must not exceed maximum."
                return
            }
            if min == nil && max == nil {
                onApply(nullMode.filter(column: column).map { [$0] } ?? [])
            } else {
                onApply([.range(column: column, min: min, max: max)])
            }
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func parse(_ text: String) throws -> Bound? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { return nil }
        if temporal { return try doc.parseValue(column: column, text: text) }
        if let value = Int64(text) { return .int(value: value) }
        if let value = UInt64(text) { return .uInt(value: value) }
        if let value = Double(text), value.isFinite { return .float(value: value) }
        throw NSError(domain: "Gewu.Filter", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "Enter a valid finite number."])
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
