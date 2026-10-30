import SwiftUI

struct InspectorSidebar: View {
    @ObservedObject var model: DocumentModel

    var body: some View {
        VStack(spacing: 0) {
            Picker("Sidebar Content", selection: $model.inspectorPage) {
                Text("Overview").tag(InspectorPage.column)
                Text("Chart").tag(InspectorPage.chart)
            }
            .pickerStyle(.segmented).controlSize(.small).labelsHidden()
            .padding(.horizontal, 12).padding(.vertical, 8)
            Divider()
            if model.inspectorPage == .column {
                if !model.info.columns.isEmpty {
                    let info = model.info
                    VStack(alignment: .leading, spacing: 4) {
                        Menu {
                            Picker("Column", selection: $model.inspectedColumn) {
                                ForEach(info.columns.indices, id: \.self) { index in
                                    Text(info.columns[index].name).tag(Optional(index))
                                }
                            }
                            .pickerStyle(.inline).labelsHidden()
                        } label: {
                            Text(selectedColumnName)
                                .font(.headline).lineLimit(1).truncationMode(.middle)
                        }
                        .menuStyle(.borderlessButton)
                        .menuIndicator(.visible)
                        .accessibilityLabel("Column")
                        .accessibilityValue(selectedColumnName)
                        .help("Choose the column to analyze")
                        if let column = model.inspectedColumn, info.columns.indices.contains(column) {
                            Text(info.columns[column].typeName)
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading).padding(12)
                    if let column = model.inspectedColumn, info.columns.indices.contains(column) {
                        ColumnAnalysisView(model: model, column: column)
                            .id(column)
                    }
                } else {
                    ContentUnavailableView("No Columns", systemImage: "tablecells")
                }
            } else {
                ScatterPanel(model: model)
            }
            Spacer(minLength: 0)
        }
    }

    private var selectedColumnName: String {
        guard let column = model.inspectedColumn, model.info.columns.indices.contains(column)
        else { return "Choose Column" }
        return model.info.columns[column].name
    }
}

/// Only visible, expanded sections request work from Core.
struct ColumnAnalysisView: View {
    @ObservedObject var model: DocumentModel
    let column: Int
    @StateObject private var analysis = InspectorModel()
    @State private var expanded: Int
    @State private var retry = 0
    @State private var histogramBins: UInt32 = 40
    @State private var histogramScale: AxisScale = .linear
    @State private var frequencyScale: AxisScale = .linear
    @AppStorage("StatisticsSignificantDigits") private var significantDigits = 8

    init(model: DocumentModel, column: Int) {
        self.model = model
        self.column = column
        let numeric = model.info.columns[column].capabilities.histogram
        _expanded = State(initialValue: numeric ? 7 : 9)
    }

    private struct Request: Equatable {
        let document: ObjectIdentifier
        let version: UInt64
        let column: Int
        let expanded: Int
        let retry: Int
        let histogramBins: UInt32
        let histogramScale: AxisScale
    }

    private var request: Request {
        Request(
            document: ObjectIdentifier(model.doc), version: model.current.version,
            column: column, expanded: expanded, retry: retry, histogramBins: histogramBins,
            histogramScale: histogramScale)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                if analysis.matches(document: model.doc, version: model.current.version, column: UInt32(column)) {
                    if let error = analysis.error { failure(error) }
                    if case .overview(let rows, let nulls) = analysis.results[.overview] {
                        Text("Missing \(countWithPercent(nulls, total: rows))")
                            .font(.caption).foregroundStyle(.secondary)
                            .help(
                                "Missing values in the current results. Column type: \(model.info.columns[column].typeName)"
                            )
                    }
                    if let error = analysis.errors[.overview] { failure(error) }
                    ForEach(analysis.sections.filter { $0 != .overview }, id: \.self) { section in
                        DisclosureGroup(isExpanded: expansion(section)) {
                            VStack(alignment: .leading, spacing: 8) {
                                if section == .distribution || section == .lengths {
                                    if section == .distribution, model.info.columns[column].capabilities.logarithmic {
                                        ScalePicker(title: "Binning", selection: $histogramScale)
                                            .help(
                                                "Log10 bins include only finite positive values. Source values remain unchanged."
                                            )
                                    }
                                    ScalePicker(title: "Frequency", selection: $frequencyScale)
                                    Picker("Maximum bins", selection: $histogramBins) {
                                        ForEach([10, 20, 40, 60, 100] as [UInt32], id: \.self) { bins in
                                            Text(bins.formatted()).tag(bins)
                                        }
                                    }
                                    .controlSize(.small)
                                    .help(
                                        "Discrete data may use fewer bins when there are fewer distinct integer positions."
                                    )
                                }
                                if analysis.loading == section { ProgressView().controlSize(.small) }
                                if let error = analysis.errors[section] { failure(error) }
                                if let result = analysis.results[section] { content(result) }
                            }
                            .padding(.top, 6).frame(maxWidth: .infinity, alignment: .leading)
                        } label: {
                            Text(section.title).font(.subheadline.weight(.semibold))
                        }
                        Divider()
                    }
                } else {
                    ProgressView().controlSize(.small)
                }
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: request) {
            await analysis.load(
                document: model.doc, version: model.current.version,
                column: UInt32(column), expanded: expanded, bins: histogramBins, scale: histogramScale)
        }
        .onDisappear { analysis.cancel() }
    }

    private func expansion(_ section: AnalysisSection) -> Binding<Bool> {
        Binding(
            get: { expanded & section.mask != 0 },
            set: { open in
                if open { expanded |= section.mask } else { expanded &= ~section.mask }
            })
    }

    private func failure(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(message).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            Button("Retry") { retry += 1 }.controlSize(.small)
        }
    }

    @ViewBuilder private func content(_ data: AnalysisData) -> some View {
        switch data {
        case .overview:
            EmptyView()
        case .statistics(let statistics):
            if let min = statistics.minLabel { detail("Min", min) }
            if let max = statistics.maxLabel { detail("Max", max) }
            if let median = statistics.medianLabel { detail("Median", median) }
            if let mean = statistics.mean { detail("Mean", value(mean)) }
            if model.kinds.indices.contains(column), model.kinds[column] == .number, let sd = statistics.stdDev {
                detail("Std dev", decimal(sd))
            }
            if statistics.nanCount > 0 { detail("NaN", formatCount(statistics.nanCount)) }
            if statistics.infiniteCount > 0 { detail("Infinite", formatCount(statistics.infiniteCount)) }
            if statistics.minLabel == nil && statistics.maxLabel == nil {
                Text("No numeric values").font(.caption).foregroundStyle(.secondary)
            }
        case .distribution(let histogram):
            distribution(histogram)
        case .values(let summary):
            detail("Distinct", (summary.estimated ? "≈ " : "") + formatCount(summary.distinctCount))
            if !summary.values.isEmpty {
                Text(summary.estimated ? "Common values · estimated" : "Values")
                    .font(.caption).foregroundStyle(.secondary)
                    .help(
                        "Candidate selection may be approximate; counts use all rows in the current results. Missing values are excluded."
                    )
                ForEach(summary.values, id: \.key) { item in
                    VStack(alignment: .leading, spacing: 3) {
                        detail(item.label ?? "(null)", countWithPercent(item.count, total: model.current.rowCount))
                        ProgressView(value: Double(item.count), total: Double(max(model.current.rowCount, 1)))
                            .tint(.accentColor)
                    }
                }
            }
        case .lengths(let summary):
            detail("Unit", summary.unit.label)
            detail("Empty", formatCount(summary.emptyCount))
            if let min = summary.min { detail("Min", formatCount(min)) }
            if let max = summary.max { detail("Max", formatCount(max)) }
            if let mean = summary.mean { detail("Mean", decimal(mean)) }
            distribution(summary.histogram)
        case .structure(let fields):
            ForEach(Array(fields.enumerated()), id: \.offset) { _, field in
                detail(field.name, field.typeName + (field.nullable ? " · nullable" : ""))
            }
        }
    }

    @ViewBuilder private func distribution(_ histogram: Histogram) -> some View {
        if let lo = histogram.lo, let hi = histogram.hi {
            HistogramChart(
                histogram: histogram,
                selection: .constant(.init(lower: lo, upper: hi)), interactive: false,
                frequencyScale: frequencyScale
            )
            .frame(height: 110).accessibilityLabel("Distribution")
            if let min = histogram.minLabel, let max = histogram.maxLabel,
                let context = ChartLabels.context(min, max)
            {
                Text(context).font(.caption2).foregroundStyle(.secondary).textSelection(.enabled)
            }
        } else {
            Text(histogram.scale == .log10 ? "No positive finite values to plot" : "No values to plot")
                .font(.caption).foregroundStyle(.secondary)
        }
        HistogramOmissions(histogram: histogram)
    }

    private func detail(_ title: String, _ text: String) -> some View {
        StatisticRow(title: title, value: text)
    }

    private func countWithPercent(_ count: UInt64, total: UInt64) -> String {
        formatCount(count) + (total == 0 ? "" : String(format: " (%.1f%%)", Double(count) / Double(total) * 100))
    }

    private func value(_ value: Double) -> String {
        if model.kinds.indices.contains(column), model.kinds[column] == .temporal,
            let integer = Int64(exactly: value.rounded()),
            let text = try? model.doc.formatValue(column: UInt32(column), value: integer)
        {
            return "≈ " + text
        }
        return decimal(value)
    }

    private func decimal(_ value: Double) -> String {
        value.formatted(.number.precision(.significantDigits(1...min(max(significantDigits, 1), 17))))
    }
}

/// Exact values remain available as selectable text even in a narrow sidebar.
struct StatisticRow: View {
    let title: String
    let value: String

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(title).foregroundStyle(.secondary).fixedSize()
                Spacer(minLength: 0)
                Text(value).fixedSize()
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(title).foregroundStyle(.secondary)
                Text(value).fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.caption).textSelection(.enabled).help(value)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension LengthUnit {
    fileprivate var label: String {
        switch self {
        case .characters: "Unicode scalars"
        case .bytes: "Bytes"
        case .elements: "Elements"
        }
    }
}
