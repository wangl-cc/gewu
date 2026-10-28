import SwiftUI

/// SwiftUI owns the inspector's presentation; the Rust document owns its statistics.
struct InspectorView: View {
    let doc: GewuDocument
    let info: FileInfo
    let kinds: [ColumnKind]
    let state: ViewState
    let revision: UInt64
    @Binding var selectedColumn: Int?
    let filteredColumns: Set<Int>
    let onSelectColumn: (Int) -> Void

    @State private var query = ""
    @State private var summary: ColumnSummary?
    @State private var histogram: Histogram?
    @State private var error: String?
    @State private var loading = false

    private struct Request: Hashable {
        let column: Int?
        let revision: UInt64
    }

    private var shownColumns: [Int] {
        info.columns.indices.filter {
            query.isEmpty || info.columns[$0].name.localizedCaseInsensitiveContains(query)
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Columns").font(.headline)
                TextField("Find column", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityLabel("Find column")
                List(selection: $selectedColumn) {
                    ForEach(shownColumns, id: \.self) { column in
                        HStack {
                            Text(info.columns[column].name).lineLimit(1)
                            if filteredColumns.contains(column) {
                                Image(systemName: "line.3.horizontal.decrease.circle.fill")
                                    .foregroundStyle(.tint)
                                    .accessibilityLabel("Filtered")
                            }
                            Spacer(minLength: 4)
                            Text(shortTypeName(info.columns[column].typeName))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        .tag(column)
                    }
                }
                .listStyle(.plain)
                .frame(height: 150)
                .onChange(of: selectedColumn) { _, column in
                    if let column { onSelectColumn(column) }
                }
                Divider()
                if let column = selectedColumn, info.columns.indices.contains(column) {
                    Text(info.columns[column].name).font(.headline)
                    Text(info.columns[column].typeName + (info.columns[column].nullable ? " · nullable" : ""))
                        .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
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
                } else {
                    Text("Select a column to inspect it").foregroundStyle(.secondary)
                }
                Divider()
                Text("File").font(.headline)
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                    detail("Name", (info.path as NSString).lastPathComponent)
                    detail("Rows", formatCount(info.rowCount))
                    detail("Size", ByteCountFormatter.string(fromByteCount: Int64(clamping: info.fileSize), countStyle: .file))
                    detail("Row groups", formatCount(UInt64(info.rowGroups)))
                    detail("Compression", info.compression.joined(separator: ", "))
                    if let createdBy = info.createdBy { detail("Created by", createdBy) }
                    ForEach(Array(info.metadata.enumerated()), id: \.offset) { _, item in
                        detail(item.key, item.value)
                    }
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: Request(column: selectedColumn, revision: revision)) { await loadSummary() }
        .onAppear {
            if selectedColumn == nil, !info.columns.isEmpty { selectedColumn = 0 }
        }
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
            if kinds[column] == .number, let sd = s.stdDev { detail("Std dev", Self.decimal(sd)) }
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
        if kinds[column] == .temporal, let integer = Int64(exactly: value.rounded()),
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
        guard let column = selectedColumn, info.columns.indices.contains(column) else { return }
        loading = true
        let numeric = kinds[column] != .text && !info.columns[column].typeName.hasPrefix("Interval")
        do {
            let doc = doc, state = state
            let result = try await Task.detached { () -> (ColumnSummary, Histogram?) in
                let summary = try doc.columnSummary(column: UInt32(column), top: 8)
                let histogram = numeric ? try doc.histogram(column: UInt32(column), state: state, bins: 40) : nil
                return (summary, histogram)
            }.value
            guard !Task.isCancelled else { return }
            summary = result.0
            histogram = result.1
            loading = false
        } catch {
            guard !Task.isCancelled else { return }
            self.error = error.localizedDescription
            loading = false
        }
    }
}
