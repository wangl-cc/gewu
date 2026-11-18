import SwiftUI

struct InspectorSidebar: View {
    @ObservedObject var model: DocumentModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            InspectorPageControl(selection: $model.inspectorPage)
                .frame(width: 180)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 16).padding(.vertical, 10)
            if model.inspectorPage == .chart {
                ScatterPanel(model: model)
            } else if let column = model.inspectedColumn, model.info.columns.indices.contains(column) {
                ColumnAnalysisView(model: model, column: column)
            } else {
                ContentUnavailableView("No Columns", systemImage: "tablecells")
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

/// Two equal native segments, kept compact independently of sidebar width.
private struct InspectorPageControl: NSViewRepresentable {
    @Binding var selection: InspectorPage

    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }

    func makeNSView(context: Context) -> NSSegmentedControl {
        let control = NSSegmentedControl(
            labels: InspectorPage.allCases.map(\.rawValue), trackingMode: .selectOne,
            target: context.coordinator, action: #selector(Coordinator.selectPage(_:)))
        control.segmentDistribution = .fill
        control.segmentStyle = .rounded
        control.controlSize = .small
        control.setContentHuggingPriority(.defaultLow, for: .horizontal)
        control.setAccessibilityLabel("Inspector")
        return control
    }

    func updateNSView(_ control: NSSegmentedControl, context: Context) {
        context.coordinator.selection = $selection
        control.selectedSegment = InspectorPage.allCases.firstIndex(of: selection) ?? 0
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSSegmentedControl, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? nsView.intrinsicContentSize.width, height: nsView.intrinsicContentSize.height)
    }

    final class Coordinator: NSObject {
        var selection: Binding<InspectorPage>
        init(selection: Binding<InspectorPage>) { self.selection = selection }

        @objc func selectPage(_ sender: NSSegmentedControl) {
            guard InspectorPage.allCases.indices.contains(sender.selectedSegment) else { return }
            selection.wrappedValue = InspectorPage.allCases[sender.selectedSegment]
        }
    }
}

/// Only visible, expanded sections request work from Core.
struct ColumnAnalysisView: View {
    @ObservedObject var model: DocumentModel
    let column: Int
    @StateObject private var analysis = InspectorModel()
    @ObservedObject private var settings: InspectorSettings
    @State private var retry = 0
    @AppStorage("StatisticsSignificantDigits") private var significantDigits = 8

    init(model: DocumentModel, column: Int) {
        self.model = model
        self.column = column
        _settings = ObservedObject(wrappedValue: model.inspectorSettings)
    }

    private var expanded: Int {
        let distribution = (try? model.dataset.analysisCapabilities(column: UInt32(column)).distribution) ?? false
        return settings.expanded(distribution: distribution)
    }

    private var histogramScale: AxisScale {
        settings.binningScale(logarithmic: model.info.columns[column].capabilities.logarithmic)
    }

    private struct Request: Equatable {
        let view: ObjectIdentifier
        let column: Int
        let expanded: Int
        let retry: Int
        let histogramBins: UInt32
        let histogramScale: AxisScale
    }

    private var request: Request {
        Request(
            view: ObjectIdentifier(model.applied.rowView),
            column: column, expanded: expanded, retry: retry, histogramBins: settings.histogramBins,
            histogramScale: histogramScale)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                InspectorColumnPicker(
                    label: "Analyzed column",
                    columns: model.info.columns.indices.map { ($0, model.info.columns[$0].name) },
                    selection: $model.inspectedColumn
                )
                Text(model.info.columns[column].displayType)
                    .font(.caption).foregroundStyle(.secondary)
                    .help(model.info.columns[column].technicalType)
            }
            .padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 12)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if analysis.matches(view: model.applied.rowView, column: UInt32(column)) {
                        VStack(alignment: .leading, spacing: 6) {
                            detail("Rows", formatCount(analysis.rowCount))
                            if let count = analysis.nullCount {
                                detail("Missing", countWithPercent(count, total: analysis.rowCount))
                            }
                        }
                        if let error = analysis.error { failure(error) }
                        if let error = analysis.errors[.overview] { failure(error) }
                        ForEach(analysis.sections.filter { $0 != .overview }, id: \.self) { section in
                            DisclosureGroup(isExpanded: expansion(section)) {
                                VStack(alignment: .leading, spacing: 6) {
                                    if section == .distribution || section == .lengths {
                                        chartOptions(section)
                                    }
                                    if analysis.loading == section { ProgressView().controlSize(.small) }
                                    if let error = analysis.errors[section] { failure(error) }
                                    content(section)
                                }
                                .padding(.top, 6).frame(maxWidth: .infinity, alignment: .leading)
                            } label: {
                                Text(section.title).font(.subheadline.weight(.semibold))
                            }
                        }
                    } else {
                        ProgressView().controlSize(.small)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .task(id: request) {
            await analysis.load(
                dataset: model.dataset, viewer: model.viewer,
                column: UInt32(column), expanded: expanded, bins: settings.histogramBins,
                scale: histogramScale)
        }
        .onDisappear { analysis.cancel() }
    }

    private func expansion(_ section: InspectorSection) -> Binding<Bool> {
        Binding(
            get: { expanded & section.mask != 0 },
            set: { open in
                settings.setExpanded(section, open)
            })
    }

    private func failure(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(message).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            Button("Retry") { retry += 1 }.controlSize(.small)
        }
    }

    @ViewBuilder private func content(_ section: InspectorSection) -> some View {
        switch section {
        case .overview:
            EmptyView()
        case .statistics:
            if let statistics = analysis.statistics {
                if let min = statistics.minLabel { detail("Min", min) }
                if let max = statistics.maxLabel { detail("Max", max) }
                if let median = statistics.medianLabel { detail("Median", median) }
                if let label = statistics.meanLabel {
                    detail("Mean", "≈ " + label)
                } else if let mean = statistics.mean {
                    detail("Mean", decimal(mean))
                }
                if model.kinds.indices.contains(column), model.kinds[column] == .number, let sd = statistics.stdDev {
                    detail("Std dev", decimal(sd))
                }
                if statistics.nanCount > 0 { detail("NaN", formatCount(statistics.nanCount)) }
                if statistics.infiniteCount > 0 { detail("Infinite", formatCount(statistics.infiniteCount)) }
                if statistics.minLabel == nil && statistics.maxLabel == nil {
                    Text("No numeric values").font(.caption).foregroundStyle(.secondary)
                }
            }
        case .distribution:
            if let histogram = analysis.distribution {
                distribution(histogram)
            }
        case .values:
            if let summary = analysis.values {
                detail("Distinct", "≈ " + formatCount(summary.distinctCount))
                    .help("Estimated distinct values in the current results")
                if !summary.values.isEmpty {
                    Text("Common values · estimated")
                        .font(.caption).foregroundStyle(.secondary)
                        .help(
                            "Candidate selection may be approximate; counts use all rows in the current results. Missing values are excluded."
                        )
                    ForEach(Array(summary.values.enumerated()), id: \.offset) { _, item in
                        FrequencyRow(label: item.label, count: item.count, total: analysis.rowCount)
                    }
                }
            }
        case .lengths:
            if let summary = analysis.lengths {
                detail("Unit", summary.unit.label)
                detail("Empty", formatCount(summary.emptyCount))
                if let min = summary.min { detail("Min", formatCount(min)) }
                if let max = summary.max { detail("Max", formatCount(max)) }
                if let mean = summary.mean { detail("Mean", decimal(mean)) }
                distribution(summary.histogram)
            }
        case .structure:
            if let fields = analysis.structure {
                ForEach(Array(fields.enumerated()), id: \.offset) { _, field in
                    detail(
                        field.name,
                        field.displayType + (field.nullable ? " · nullable" : ""),
                        help: field.technicalType
                    )
                }
            }
        }
    }

    private func chartOptions(_ section: InspectorSection) -> some View {
        HStack {
            Text(section == .distribution && histogramScale == .log10 ? "Logarithmic bins" : "Linear bins")
            if settings.frequencyScale == .log10 { Text("· Log frequency") }
            Spacer(minLength: 0)
            Menu {
                if section == .distribution, model.info.columns[column].capabilities.logarithmic {
                    Picker("Bin spacing", selection: $settings.histogramScale) {
                        Text("Linear").tag(AxisScale.linear)
                        Text("Logarithmic").tag(AxisScale.log10)
                    }
                }
                Picker("Frequency", selection: $settings.frequencyScale) {
                    Text("Linear").tag(AxisScale.linear)
                    Text("Logarithmic").tag(AxisScale.log10)
                }
                Picker("Maximum bins", selection: $settings.histogramBins) {
                    ForEach([10, 20, 40, 60, 100] as [UInt32], id: \.self) { Text($0.formatted()).tag($0) }
                }
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton).fixedSize().accessibilityLabel("Chart display options")
        }.font(.caption2).foregroundStyle(.secondary)
    }

    @ViewBuilder private func distribution(_ histogram: Histogram) -> some View {
        if let lo = histogram.lo, let hi = histogram.hi {
            HistogramChart(
                histogram: histogram,
                selection: .constant(.init(lower: lo, upper: hi)), interactive: false,
                frequencyScale: settings.frequencyScale
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

    private func detail(_ title: String, _ text: String, help: String? = nil) -> some View {
        StatisticRow(title: title, value: text, help: help)
    }

    private func countWithPercent(_ count: UInt64, total: UInt64) -> String {
        formatCount(count) + (total == 0 ? "" : String(format: " (%.1f%%)", Double(count) / Double(total) * 100))
    }

    private func decimal(_ value: Double) -> String {
        value.formatted(.number.precision(.significantDigits(1...min(max(significantDigits, 1), 17))))
    }
}

private struct FrequencyRow: View {
    let label: String
    let count: UInt64
    let total: UInt64

    var body: some View {
        VStack(spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(label.isEmpty ? "(empty string)" : label).lineLimit(1).help(label)
                Spacer(minLength: 8)
                Text(formatCount(count)).monospacedDigit()
                Text(total == 0 ? "0%" : String(format: "%.1f%%", Double(count) / Double(total) * 100))
                    .foregroundStyle(.secondary).monospacedDigit().frame(width: 44, alignment: .trailing)
            }.font(.caption).textSelection(.enabled)
            GeometryReader { geometry in
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(Color.accentColor.opacity(0.5))
                    .frame(width: geometry.size.width * min(1, Double(count) / Double(max(total, 1))))
            }.frame(height: 3)
        }.padding(.vertical, 3)
    }
}

/// Exact values remain available as selectable text even in a narrow sidebar.
struct StatisticRow: View {
    let title: String
    let value: String
    var help: String? = nil

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(title).foregroundStyle(.secondary).fixedSize().frame(minWidth: 70, alignment: .leading)
                Text(value).fixedSize()
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(title).foregroundStyle(.secondary)
                Text(value).fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.callout).monospacedDigit().textSelection(.enabled).help(help ?? value)
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
