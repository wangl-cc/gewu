import GewuKit
import Observation
import SwiftUI

struct ScatterPanel: View {
    @Bindable var model: TableModel
    @Bindable var settings: InspectorSettings
    @State private var chart = ScatterModel()
    @State private var hovered: ScatterPoint?
    @State private var attempt = 0
    @AppStorage(Preferences.rowNumberBase) private var rowNumberBase

    private var columns: [ColumnID] {
        model.visibleColumns.filter { model.column($0)?.capabilities.scatter == true }
    }

    private var request: ScatterModel.Request? {
        guard let y = model.chartY, columns.contains(y) else { return nil }
        return ScatterModel.Request(
            rows: model.displayed.rowSet, x: model.chartX, y: y, limit: settings.scatterPointLimit,
            xScale: supportsLog(model.chartX) ? settings.scatterXScale : .linear,
            yScale: supportsLog(model.chartY) ? settings.scatterYScale : .linear,
            rowNumberBase: rowNumberBase, attempt: attempt)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !columns.isEmpty {
                Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                    GridRow {
                        Text("X").font(.caption).foregroundStyle(.secondary)
                        axisPicker("X", selection: $model.chartX, allowsSourceRow: true)
                    }
                    GridRow {
                        Text("Y").font(.caption).foregroundStyle(.secondary)
                        axisPicker("Y", selection: $model.chartY, allowsSourceRow: false)
                    }
                }
                .padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 12)
                Divider()
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if columns.isEmpty {
                        ContentUnavailableView("No Plottable Columns", systemImage: "chart.dots.scatter")
                    } else {
                        HStack {
                            Text("Scatter plot").font(.subheadline.weight(.semibold))
                            Spacer()
                            Menu {
                                if supportsLog(model.chartX) {
                                    Picker("X axis", selection: $settings.scatterXScale) {
                                        Text("Linear").tag(AxisScale.linear)
                                        Text("Logarithmic").tag(AxisScale.log10)
                                    }
                                }
                                if supportsLog(model.chartY) {
                                    Picker("Y axis", selection: $settings.scatterYScale) {
                                        Text("Linear").tag(AxisScale.linear)
                                        Text("Logarithmic").tag(AxisScale.log10)
                                    }
                                }
                                Picker("Maximum points", selection: $settings.scatterPointLimit) {
                                    ForEach([512, 1024, 2048, 4096] as [UInt32], id: \.self) { limit in
                                        Text(limit.formatted()).tag(limit)
                                    }
                                }
                            } label: {
                                Image(systemName: "ellipsis")
                            }
                            .menuStyle(.borderlessButton).fixedSize().accessibilityLabel("Plot display options")
                        }.controlSize(.small)
                        if let request, request.xScale == .log10 || request.yScale == .log10 {
                            Text(
                                "Logarithmic: \([request.xScale == .log10 ? "X" : nil, request.yScale == .log10 ? "Y" : nil].compactMap { $0 }.joined(separator: ", "))"
                            )
                            .font(.caption).foregroundStyle(.secondary)
                        }
                        if chart.loading { ProgressView().controlSize(.small) }
                        if let error = chart.error {
                            Text(error).font(.caption).foregroundStyle(.red)
                            Button("Retry") { attempt += 1 }
                        }
                        if let result = chart.result {
                            if result.points.isEmpty {
                                Text(
                                    chart.xScale == .log10 || chart.yScale == .log10
                                        ? "No finite pairs in the selected log domain" : "No finite pairs to plot"
                                ).foregroundStyle(.secondary)
                            } else {
                                ScatterChart(
                                    points: result.points, xDates: chart.xDates, yDates: chart.yDates,
                                    rowNumberBase: rowNumberBase, xScale: chart.xScale, yScale: chart.yScale,
                                    hovered: $hovered
                                )
                                .frame(height: 220)
                                .accessibilityLabel("Scatter plot, \(result.points.count) points")
                                .accessibilityIdentifier("scatterPlot")
                            }
                            VStack(alignment: .leading, spacing: 4) {
                                if !result.points.isEmpty {
                                    Text(
                                        result.isSampled
                                            ? "\(formatCount(UInt64(result.points.count))) sampled of \(formatCount(result.eligibleRows)) pairs"
                                            : "\(formatCount(result.eligibleRows)) pairs")
                                }
                                let invalidRows = chart.totalRows - result.eligibleRows - result.nonPositiveRows
                                if invalidRows > 0 {
                                    Text("\(formatCount(invalidRows)) rows omitted: missing or non-finite values")
                                }
                                if result.nonPositiveRows > 0 {
                                    Text(
                                        "\(formatCount(result.nonPositiveRows)) rows omitted: zero or negative on a log axis"
                                    )
                                }
                            }.font(.caption).foregroundStyle(.secondary)
                            if !result.points.isEmpty {
                                if let dates = chart.xDates {
                                    Text("X: \(ScatterDates.context(dates))")
                                        .font(.caption2).foregroundStyle(.secondary)
                                        .help("Hover details retain the original date/time text and precision.")
                                }
                                if let dates = chart.yDates {
                                    Text("Y: \(ScatterDates.context(dates))")
                                        .font(.caption2).foregroundStyle(.secondary)
                                        .help("Hover details retain the original date/time text and precision.")
                                }
                                VStack(alignment: .leading, spacing: 4) {
                                    if let point = hovered {
                                        StatisticRow(
                                            title: "Source row",
                                            value: formatRowNumber(point.sourceRow, base: rowNumberBase)
                                        )
                                        .accessibilityElement(children: .contain)
                                        .accessibilityIdentifier("scatterHoveredRow")
                                        StatisticRow(title: "X", value: point.xLabel)
                                        StatisticRow(title: "Y", value: point.yLabel)
                                    } else {
                                        Text("Point to a dot to see its values")
                                            .font(.caption2).foregroundStyle(.secondary)
                                    }
                                }.frame(minHeight: 54, alignment: .topLeading)
                            }
                        }
                    }
                }
                .padding(16).frame(maxWidth: .infinity, alignment: .topLeading)
            }
        }
        .onChange(of: model.chartX) { _, column in
            if !supportsLog(column) { settings.scatterXScale = .linear }
        }
        .onChange(of: model.chartY) { _, column in
            if !supportsLog(column) { settings.scatterYScale = .linear }
        }
        .task(id: request) {
            hovered = nil
            await chart.load(request)
        }
    }

    private func axisPicker(_ axis: String, selection: Binding<ColumnID?>, allowsSourceRow: Bool) -> some View {
        InspectorColumnPicker(
            label: "\(axis) axis",
            columns: columns.compactMap { id in model.column(id).map { (id, $0.name) } },
            emptySelectionTitle: allowsSourceRow ? "Source row" : nil, selection: selection
        ).frame(maxWidth: .infinity)
    }

    private func supportsLog(_ column: ColumnID?) -> Bool {
        column.flatMap { model.column($0)?.capabilities.logarithmic } ?? false
    }
}

/// The sample shown for one scatter request. The row set computes and caches
/// the sample; this model adds presentation only. The panel runs
/// ``load(_:)`` in `.task(id:)`, so a new request cancels the previous one.
@MainActor
@Observable
final class ScatterModel {
    struct Request: Equatable {
        let rows: RowSet
        let x: ColumnID?
        let y: ColumnID
        let limit: UInt32
        let xScale: AxisScale
        let yScale: AxisScale
        let rowNumberBase: Int
        /// Changed by Retry, to run the same request again.
        let attempt: Int
    }

    private(set) var result: ScatterResult?
    private(set) var totalRows: UInt64 = 0
    private(set) var xDates: [Date]?
    private(set) var yDates: [Date]?
    private(set) var xScale: AxisScale = .linear
    private(set) var yScale: AxisScale = .linear
    private(set) var loading = false
    private(set) var error: String?

    func load(_ request: Request?) async {
        result = nil
        error = nil
        guard let request else { return }
        loading = true
        defer { loading = false }
        do {
            var sample = try await request.rows.scatter(
                x: request.x, y: request.y, limit: request.limit, xScale: request.xScale, yScale: request.yScale)
            try Task.checkCancellation()
            // Row numbering follows display settings, so only the raw sample is cached.
            if request.x == nil {
                for index in sample.points.indices {
                    let row = sample.points[index].sourceRow
                    sample.points[index].x = Double(rowNumber(row, base: request.rowNumberBase))
                    sample.points[index].xLabel = formatRowNumber(row, base: request.rowNumberBase)
                }
            }
            xDates = ScatterDates.make(points: sample.points, x: true, kind: sample.xAxisKind)
            yDates = ScatterDates.make(points: sample.points, x: false, kind: sample.yAxisKind)
            xScale = request.xScale
            yScale = request.yScale
            totalRows = UInt64(request.rows.count)
            result = sample
        } catch {
            guard !Task.isCancelled, !(error is CancellationError) else { return }
            self.error = ErrorPresentation(error).text
        }
    }
}
