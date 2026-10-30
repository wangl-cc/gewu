import SwiftUI

struct ScatterPanel: View {
    @ObservedObject var model: DocumentModel
    @StateObject private var chart = ScatterModel()
    @State private var hovered: ScatterPoint?
    @State private var retry = 0
    @State private var pointLimit: UInt32 = 2048
    @AppStorage("RowNumberBase") private var rowNumberBase = 0

    private var columns: [Int] {
        model.info?.columns.indices.filter { model.info?.columns[$0].capabilities.scatter == true } ?? []
    }

    private struct Request: Equatable {
        let document: ObjectIdentifier?
        let version: UInt64
        let x: Int?
        let y: Int?
        let retry: Int
        let pointLimit: UInt32
        let rowNumberBase: Int
    }

    private var request: Request {
        Request(
            document: model.doc.map(ObjectIdentifier.init), version: model.current.version,
            x: model.chartX, y: model.chartY, retry: retry, pointLimit: pointLimit, rowNumberBase: rowNumberBase)
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
                    Picker("Y", selection: $model.chartY) {
                        if model.chartY == nil { Text("Select column").tag(Optional<Int>.none) }
                        columnOptions
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
                            Text("No finite pairs to plot").foregroundStyle(.secondary)
                        } else {
                            ScatterChart(
                                points: result.points, xDates: chart.xDates, yDates: chart.yDates, hovered: $hovered
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
                            if result.totalRows > result.eligibleRows {
                                Text(
                                    "\(formatCount(result.totalRows - result.eligibleRows)) rows omitted: missing or non-finite values"
                                )
                                .font(.caption2).foregroundStyle(.secondary)
                            }
                            if let point = hovered {
                                StatisticRow(
                                    title: "Source row",
                                    value: formatCount(point.sourceRow + UInt64(rowNumberBase == 1 ? 1 : 0)))
                                StatisticRow(title: "X", value: point.xLabel)
                                StatisticRow(title: "Y", value: point.yLabel)
                            } else {
                                Text("Point to a dot to see its values")
                                    .font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .padding(12).frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear {
            if model.chartY == nil { model.chartY = columns.first }
        }
        .task(id: request) {
            hovered = nil
            guard let document = model.doc, let y = model.chartY else { return }
            await chart.load(
                document: document, version: model.current.version, x: model.chartX, y: y,
                xKind: model.chartX.flatMap { model.info?.columns[$0].kind }, yKind: model.info?.columns[y].kind,
                limit: pointLimit, rowNumberBase: rowNumberBase)
        }
        .onDisappear { chart.cancel() }
    }

    private var columnOptions: some View {
        ForEach(columns, id: \.self) { index in
            Text(model.info?.columns[index].name ?? "").tag(Optional(index))
        }
    }

}

@MainActor
final class ScatterModel: ObservableObject {
    @Published private(set) var result: ScatterResult?
    private(set) var xDates: [Date]?
    private(set) var yDates: [Date]?
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
        rowNumberBase: Int = 0
    ) async {
        cancel()
        let ticket = request
        result = nil
        error = nil
        loading = true
        do {
            try Task.checkCancellation()
            let work = try document.prepareScatter(
                version: version, xColumn: x.map(UInt32.init), yColumn: UInt32(y), limit: limit)
            job = work
            let loaded = try await withTaskCancellationHandler {
                try await Task.detached(priority: .userInitiated) {
                    var result = try work.run()
                    try Task.checkCancellation()
                    if x == nil && rowNumberBase == 1 {
                        for index in result.points.indices {
                            result.points[index].x += 1
                            result.points[index].xLabel = String(result.points[index].sourceRow + 1)
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
            result = loaded.0
        } catch {
            guard !Task.isCancelled, ticket == request else { return }
            switch error {
            case GewuError.Cancelled, GewuError.StaleVersion: break
            default: self.error = error.localizedDescription
            }
        }
        guard ticket == request else { return }
        loading = false
        job = nil
    }
}
