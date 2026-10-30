import SwiftUI

struct ScatterPanel: View {
    @ObservedObject var model: DocumentModel
    @StateObject private var chart = ScatterModel()
    @State private var hovered: ScatterPoint?
    @State private var retry = 0
    @State private var pointLimit: UInt32 = 2048
    @State private var xScale: AxisScale = .linear
    @State private var yScale: AxisScale = .linear
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    private var columns: [Int] {
        model.info.columns.indices.filter { model.info.columns[$0].capabilities.scatter }
    }

    private struct Request: Equatable {
        let document: ObjectIdentifier
        let version: UInt64
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
            document: ObjectIdentifier(model.doc), version: model.current.version,
            x: model.chartX, y: model.chartY, retry: retry, pointLimit: pointLimit, rowNumberBase: rowNumberBase,
            xScale: supportsLog(model.chartX) ? xScale : .linear,
            yScale: supportsLog(model.chartY) ? yScale : .linear)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if columns.isEmpty {
                    ContentUnavailableView("No Plottable Columns", systemImage: "chart.dots.scatter")
                } else {
                    Picker("X", selection: $model.chartX) {
                        Text("Source row").tag(Optional<Int>.none)
                        columnOptions
                    }
                    if supportsLog(model.chartX) {
                        ScalePicker(title: "X scale", selection: $xScale)
                    }
                    Picker("Y", selection: $model.chartY) {
                        if model.chartY == nil { Text("Select column").tag(Optional<Int>.none) }
                        columnOptions
                    }
                    if supportsLog(model.chartY) {
                        ScalePicker(title: "Y scale", selection: $yScale)
                    }
                    Picker("Maximum points", selection: $pointLimit) {
                        ForEach([512, 1024, 2048, 4096] as [UInt32], id: \.self) { limit in
                            Text(limit.formatted()).tag(limit)
                        }
                    }
                    .help("Samples the current results when there are more valid pairs than this limit.")
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
                            Text(
                                result.sampled
                                    ? "\(formatCount(UInt64(result.points.count))) sampled of \(formatCount(result.eligibleRows)) pairs"
                                    : "\(formatCount(result.eligibleRows)) pairs"
                            )
                            .font(.caption).foregroundStyle(.secondary)
                            if let point = hovered {
                                StatisticRow(
                                    title: "Source row",
                                    value: formatRowNumber(point.sourceRow, base: rowNumberBase))
                                StatisticRow(title: "X", value: point.xLabel)
                                StatisticRow(title: "Y", value: point.yLabel)
                            } else {
                                Text("Point to a dot to see its values")
                                    .font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                        let invalidRows = result.totalRows - result.eligibleRows - result.nonPositiveRows
                        if invalidRows > 0 {
                            Text("\(formatCount(invalidRows)) rows omitted: missing or non-finite values")
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                        if result.nonPositiveRows > 0 {
                            Text("\(formatCount(result.nonPositiveRows)) rows omitted: zero or negative on a log axis")
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .padding(12).frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear {
            if model.chartY == nil { model.chartY = columns.first }
        }
        .onChange(of: model.chartX) { _, column in
            if !supportsLog(column) { xScale = .linear }
        }
        .onChange(of: model.chartY) { _, column in
            if !supportsLog(column) { yScale = .linear }
        }
        .task(id: request) {
            hovered = nil
            guard let y = model.chartY else { return }
            await chart.load(
                document: model.doc, version: model.current.version, x: model.chartX, y: y,
                xKind: model.chartX.map { model.info.columns[$0].kind }, yKind: model.info.columns[y].kind,
                limit: pointLimit, rowNumberBase: rowNumberBase,
                xScale: request.xScale, yScale: request.yScale)
        }
        .onDisappear { chart.cancel() }
    }

    private var columnOptions: some View {
        ForEach(columns, id: \.self) { index in
            Text(model.info.columns[index].name).tag(Optional(index))
        }
    }

    private func supportsLog(_ column: Int?) -> Bool {
        guard let column, model.info.columns.indices.contains(column) else { return false }
        return model.info.columns[column].capabilities.logarithmic
    }

}

@MainActor
final class ScatterModel: ObservableObject {
    @Published private(set) var result: ScatterResult?
    private(set) var xDates: [Date]?
    private(set) var yDates: [Date]?
    private(set) var xScale: AxisScale = .linear
    private(set) var yScale: AxisScale = .linear
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private var request: UInt64 = 0
    private var job: ScatterAnalysis?

    func cancel() {
        request &+= 1
        job?.cancel()
        job = nil
        loading = false
    }

    func load(
        document: GewuDocument, version: UInt64, x: Int?, y: Int, xKind: DataKind?, yKind: DataKind?,
        limit: UInt32 = 2048,
        rowNumberBase: Int = 0,
        xScale: AxisScale = .linear,
        yScale: AxisScale = .linear
    ) async {
        cancel()
        let ticket = request
        result = nil
        error = nil
        loading = true
        do {
            try Task.checkCancellation()
            let work = try document.prepareScatter(
                version: version, xColumn: x.map(UInt32.init), yColumn: UInt32(y), limit: limit,
                xScale: xScale, yScale: yScale)
            job = work
            let loaded = try await withTaskCancellationHandler {
                try await Task.detached(priority: .userInitiated) {
                    var result = try work.run()
                    try Task.checkCancellation()
                    if x == nil {
                        for index in result.points.indices {
                            let sourceRow = result.points[index].sourceRow
                            result.points[index].x = Double(rowNumber(sourceRow, base: rowNumberBase))
                            result.points[index].xLabel = formatRowNumber(sourceRow, base: rowNumberBase)
                        }
                    }
                    return (
                        result,
                        ScatterDates.make(points: result.points, x: true, kind: xKind),
                        ScatterDates.make(points: result.points, x: false, kind: yKind)
                    )
                }.value
            } onCancel: {
                work.cancel()
            }
            guard !Task.isCancelled, ticket == request else { return }
            xDates = loaded.1
            yDates = loaded.2
            self.xScale = xScale
            self.yScale = yScale
            result = loaded.0
        } catch {
            guard !Task.isCancelled, ticket == request else { return }
            switch error {
            case GewuError.Cancelled, GewuError.StaleVersion: break
            default: self.error = error.userMessage
            }
        }
        guard ticket == request else { return }
        loading = false
        job = nil
    }
}
