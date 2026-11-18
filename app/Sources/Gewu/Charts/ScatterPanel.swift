import SwiftUI

struct ScatterPanel: View {
    @ObservedObject var model: DocumentModel
    @StateObject private var chart = ScatterModel()
    @State private var hovered: ScatterPoint?
    @State private var retry = 0
    @ObservedObject private var settings: InspectorSettings
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    init(model: DocumentModel) {
        self.model = model
        _settings = ObservedObject(wrappedValue: model.inspectorSettings)
    }

    private var columns: [Int] {
        model.info.columns.indices.filter { model.info.columns[$0].capabilities.scatter }
    }

    private struct Request: Equatable {
        let view: ObjectIdentifier
        let x: Int?
        let y: Int?
        let retry: Int
        let pointLimit: UInt32
        let rowNumberBase: Int
        let xScale: AxisScale
        let yScale: AxisScale
    }

    private var request: Request {
        Request(
            view: ObjectIdentifier(model.applied.rowView),
            x: model.chartX, y: model.chartY, retry: retry, pointLimit: settings.scatterPointLimit,
            rowNumberBase: rowNumberBase,
            xScale: supportsLog(model.chartX) ? settings.scatterXScale : .linear,
            yScale: supportsLog(model.chartY) ? settings.scatterYScale : .linear)
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
                        if request.xScale == .log10 || request.yScale == .log10 {
                            Text(
                                "Logarithmic: \([request.xScale == .log10 ? "X" : nil, request.yScale == .log10 ? "Y" : nil].compactMap { $0 }.joined(separator: ", "))"
                            )
                            .font(.caption).foregroundStyle(.secondary)
                        }
                        if chart.loading { ProgressView().controlSize(.small) }
                        if let error = chart.error {
                            Text(error).font(.caption).foregroundStyle(.red)
                            Button("Retry") { retry += 1 }
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
                                        result.eligibleRows > UInt64(result.points.count)
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
        .onAppear {
            if model.chartY == nil { model.chartY = columns.first }
        }
        .onChange(of: model.chartX) { _, column in
            if !supportsLog(column) { settings.scatterXScale = .linear }
        }
        .onChange(of: model.chartY) { _, column in
            if !supportsLog(column) { settings.scatterYScale = .linear }
        }
        .task(id: request) {
            hovered = nil
            guard let y = model.chartY else { return }
            await chart.load(
                viewer: model.viewer, x: model.chartX,
                y: y,
                limit: settings.scatterPointLimit, rowNumberBase: rowNumberBase,
                xScale: request.xScale, yScale: request.yScale)
        }
        .onDisappear { chart.cancel() }
    }

    private func axisPicker(_ axis: String, selection: Binding<Int?>, allowsSourceRow: Bool) -> some View {
        InspectorColumnPicker(
            label: "\(axis) axis", columns: columns.map { ($0, model.info.columns[$0].name) },
            emptySelectionTitle: allowsSourceRow ? "Source row" : nil, selection: selection
        ).frame(maxWidth: .infinity)
    }

    private func supportsLog(_ column: Int?) -> Bool {
        guard let column, model.info.columns.indices.contains(column) else { return false }
        return model.info.columns[column].capabilities.logarithmic
    }

}

@MainActor
final class ScatterModel: ObservableObject {
    @Published private(set) var result: ScatterResult?
    private(set) var totalRows: UInt64 = 0
    private(set) var xDates: [Date]?
    private(set) var yDates: [Date]?
    private(set) var xScale: AxisScale = .linear
    private(set) var yScale: AxisScale = .linear
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private var request: UInt64 = 0
    private var work: Task<Void, Never>?

    /// Cancel this panel's pending sample and presentation work.
    func cancel() {
        request &+= 1
        work?.cancel()
        work = nil
        loading = false
    }

    func load(
        viewer: Viewer, x: Int?, y: Int,
        limit: UInt32 = 2048,
        rowNumberBase: Int = 0,
        xScale: AxisScale = .linear,
        yScale: AxisScale = .linear
    ) async {
        guard !Task.isCancelled else { return }
        cancel()
        let ticket = request
        let view = viewer.applied.rowView
        let task = Task {
            await performLoad(
                viewer: viewer, view: view, x: x, y: y, limit: limit, rowNumberBase: rowNumberBase,
                xScale: xScale, yScale: yScale, ticket: ticket)
        }
        work = task
        await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }
        if request == ticket { work = nil }
    }

    private func performLoad(
        viewer: Viewer, view: GewuView, x: Int?, y: Int, limit: UInt32, rowNumberBase: Int,
        xScale: AxisScale, yScale: AxisScale, ticket: UInt64
    ) async {
        guard !Task.isCancelled, ticket == request, viewer.applied.rowView === view else { return }
        defer {
            if ticket == request { loading = false }
        }
        result = nil
        error = nil
        loading = true
        do {
            try Task.checkCancellation()
            let rowCount = view.rowCount()
            let sample = try await viewer.scatter(
                x: x.map(UInt32.init), y: UInt32(y), limit: limit, xScale: xScale, yScale: yScale)
            try Task.checkCancellation()
            // Row numbering depends on display settings, so only the raw sample is cached.
            let loaded = try await Task.detached(priority: .userInitiated) {
                var result = sample
                if x == nil {
                    for index in result.points.indices {
                        let sourceRow = result.points[index].sourceRow
                        result.points[index].x = Double(rowNumber(sourceRow, base: rowNumberBase))
                        result.points[index].xLabel = formatRowNumber(sourceRow, base: rowNumberBase)
                    }
                }
                return (
                    result,
                    ScatterDates.make(points: result.points, x: true, kind: result.xAxisKind),
                    ScatterDates.make(points: result.points, x: false, kind: result.yAxisKind)
                )
            }.value
            try Task.checkCancellation()
            guard !Task.isCancelled, ticket == request, viewer.applied.rowView === view else { return }
            xDates = loaded.1
            yDates = loaded.2
            self.xScale = xScale
            self.yScale = yScale
            totalRows = rowCount
            result = loaded.0
        } catch {
            guard !Task.isCancelled, ticket == request else { return }
            if !(error is CancellationError) {
                self.error = error.userMessage
            }
        }
    }
}
