import SwiftUI

/// Statistics are requested only while this column's popover is open.
struct ColumnInfoView: View {
    @ObservedObject var model: DocumentModel
    let column: Int
    @State private var summary: ColumnSummary?
    @State private var histogram: Histogram?
    @State private var error: String?
    @State private var loading = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let info = model.info, info.columns.indices.contains(column) {
                    Text(info.columns[column].name).font(.headline)
                    Text(info.columns[column].typeName + (info.columns[column].nullable ? " · nullable" : ""))
                        .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    Text("Current results · filters and search included")
                        .font(.caption).foregroundStyle(.secondary)
                    Divider()
                    if loading { ProgressView().controlSize(.small) }
                    if let error { Text(error).font(.caption).foregroundStyle(.red) }
                    if let summary {
                        statistics(summary, column: column)
                    }
                    if let h = histogram, let lo = h.lo, let hi = h.hi {
                        HistogramChart(histogram: h,
                                       selection: .constant(.init(lower: lo, upper: hi)),
                                       interactive: false,
                                       labelFormatter: { value($0, column: column) })
                            .frame(height: 100)
                            .accessibilityLabel("Distribution of \(info.columns[column].name)")
                    }
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(width: 340, height: 420)
        .task(id: model.current.version) { await loadSummary() }
    }

    @ViewBuilder private func statistics(_ s: ColumnSummary, column: Int) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
            detail("Rows in view", formatCount(s.rows))
            detail("Nulls", countWithPercent(s.nullCount, total: s.rows))
            if s.nanCount > 0 { detail("NaN", countWithPercent(s.nanCount, total: s.rows)) }
            detail("Distinct", formatCount(s.distinctCount))
            if let min = s.min { detail("Min", value(min, column: column)) }
            if let max = s.max { detail("Max", value(max, column: column)) }
            if let mean = s.mean { detail("Mean", value(mean, column: column)) }
            if model.kinds[column] == .number, let sd = s.stdDev { detail("Std dev", Self.decimal(sd)) }
        }
        if !s.topValues.isEmpty {
            Text("Most common").font(.subheadline)
            ForEach(s.topValues, id: \.key) { item in
                HStack {
                    Text(item.label ?? "(null)").lineLimit(1).help(item.label ?? "(null)")
                    Spacer(minLength: 6)
                    ProgressView(value: Double(item.count), total: Double(max(s.topValues.first?.count ?? 1, 1)))
                        .frame(width: 42)
                    Text(formatCount(item.count)).monospacedDigit().foregroundStyle(.secondary)
                }
                .font(.caption)
            }
        }
    }

    private func detail(_ title: String, _ text: String) -> some View {
        GridRow(alignment: .top) {
            Text(title).foregroundStyle(.secondary)
            Text(text).textSelection(.enabled).lineLimit(3).help(text)
        }
        .font(.caption)
    }

    private func countWithPercent(_ count: UInt64, total: UInt64) -> String {
        formatCount(count) + (total == 0 ? "" : String(format: " (%.1f%%)", Double(count) / Double(total) * 100))
    }

    private func value(_ value: Double, column: Int) -> String {
        if model.kinds[column] == .temporal, let doc = model.doc, let integer = Int64(exactly: value.rounded()),
           let text = try? doc.formatValue(column: UInt32(column), value: integer) {
            return text
        }
        return Self.decimal(value)
    }

    private static func decimal(_ value: Double) -> String {
        let magnitude = abs(value)
        if magnitude != 0, magnitude >= 1e15 || magnitude < 1e-3 { return String(format: "%.4g", value) }
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 3
        return formatter.string(from: NSNumber(value: value)) ?? String(value)
    }

    @MainActor private func loadSummary() async {
        summary = nil
        histogram = nil
        error = nil
        guard let doc = model.doc, let info = model.info, info.columns.indices.contains(column) else { return }
        loading = true
        let numeric = info.columns[column].capabilities.histogram
        let revision = model.current.version
        do {
            let result = try await Task.detached { () -> (ColumnSummary, Histogram?) in
                guard doc.currentView().version == revision else { throw CancellationError() }
                let summary = try doc.columnSummary(column: UInt32(column), top: 8)
                let histogram = numeric ? try doc.viewHistogram(version: revision, column: UInt32(column), bins: 40) : nil
                guard doc.currentView().version == revision else { throw CancellationError() }
                return (summary, histogram)
            }.value
            guard !Task.isCancelled else { return }
            summary = result.0
            histogram = result.1
            loading = false
        } catch is CancellationError {
            guard !Task.isCancelled else { return }
            loading = false
        } catch GewuError.StaleVersion {
            guard !Task.isCancelled else { return }
            loading = false
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
            loading = false
        }
    }
}
