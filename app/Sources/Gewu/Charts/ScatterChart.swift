import Charts
import SwiftUI

/// Charts owns scales and layout. Core's original labels remain the
/// source of truth for hover details; plotting coordinates are approximate.
struct ScatterChart: View {
    let points: [ScatterPoint]
    let xDates: [Date]?
    let yDates: [Date]?
    let rowNumberBase: Int
    let xScale: AxisScale
    let yScale: AxisScale
    @Binding var hovered: ScatterPoint?

    var body: some View {
        Chart(points.indices, id: \.self) { index in
            point(at: index)
                .symbolSize(hovered?.sourceRow == points[index].sourceRow ? 45 : 16)
                .foregroundStyle(Color.accentColor.opacity(0.65))
                .accessibilityLabel("Row \(formatRowNumber(points[index].sourceRow, base: rowNumberBase))")
                .accessibilityValue("X: \(points[index].xLabel), Y: \(points[index].yLabel)")
        }
        .chartXScale(
            domain: .automatic(includesZero: false), range: .plotDimension(padding: 8),
            type: xScale == .log10 ? .log : nil
        )
        .chartYScale(
            domain: .automatic(includesZero: false), range: .plotDimension(padding: 8),
            type: yScale == .log10 ? .log : nil
        )
        .chartXAxis {
            if xScale == .log10 {
                logAxis(position: .bottom, values: ScatterLogTicks.values(points.lazy.map(\.x), maximumCount: 3))
            } else {
                AxisMarks(position: .bottom, values: .automatic(desiredCount: 3))
            }
        }
        .chartYAxis {
            if yScale == .log10 {
                logAxis(position: .leading, values: ScatterLogTicks.values(points.lazy.map(\.y), maximumCount: 4))
            } else {
                AxisMarks(position: .leading, values: .automatic(desiredCount: 4))
            }
        }
        .environment(\.timeZone, TimeZone(secondsFromGMT: 0)!)
        .environment(\.calendar, Self.calendar)
        .chartOverlay { proxy in
            GeometryReader { geometry in
                Rectangle().fill(.clear).contentShape(Rectangle())
                    .onContinuousHover { phase in
                        switch phase {
                        case .active(let location):
                            guard let anchor = proxy.plotFrame else { return }
                            let frame = geometry[anchor]
                            hovered =
                                frame.contains(location)
                                ? nearest(
                                    to: CGPoint(x: location.x - frame.minX, y: location.y - frame.minY), proxy: proxy)
                                : nil
                        case .ended:
                            hovered = nil
                        }
                    }
            }
        }
        .help(
            hovered.map {
                "Row \(formatRowNumber($0.sourceRow, base: rowNumberBase))\nX: \($0.xLabel)\nY: \($0.yLabel)"
            }
                ?? "Point to a dot to see its values"
        )
        .onDisappear { hovered = nil }
    }

    private func logAxis(position: AxisMarkPosition, values: [Double]) -> some AxisContent {
        AxisMarks(position: position, values: values) { value in
            AxisGridLine()
            AxisTick()
            AxisValueLabel {
                if let number = value.as(Double.self) {
                    Text(ScatterLogTicks.label(number)).font(.caption2)
                }
            }
        }
    }

    @ChartContentBuilder
    private func point(at index: Int) -> some ChartContent {
        let p = points[index]
        if let xDates, let yDates {
            PointMark(x: .value("X", xDates[index]), y: .value("Y", yDates[index]))
        } else if let xDates {
            PointMark(x: .value("X", xDates[index]), y: .value("Y", p.y))
        } else if let yDates {
            PointMark(x: .value("X", p.x), y: .value("Y", yDates[index]))
        } else {
            PointMark(x: .value("X", p.x), y: .value("Y", p.y))
        }
    }

    private func nearest(to location: CGPoint, proxy: ChartProxy) -> ScatterPoint? {
        var nearest: ScatterPoint?
        var distance = 100.0
        for index in points.indices {
            let p = points[index]
            let x = xDates.map { proxy.position(forX: $0[index]) } ?? proxy.position(forX: p.x)
            let y = yDates.map { proxy.position(forY: $0[index]) } ?? proxy.position(forY: p.y)
            guard let x, let y else { continue }
            let squared = (x - location.x) * (x - location.x) + (y - location.y) * (y - location.y)
            if squared < distance {
                nearest = p
                distance = squared
            }
        }
        return nearest
    }

    private static var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }
}

/// Native log axes otherwise label every decade, regardless of desiredCount.
/// Tick values stay in source units; only their labels use compact notation.
enum ScatterLogTicks {
    static func values<S: Sequence>(_ values: S, maximumCount: Int) -> [Double] where S.Element == Double {
        guard maximumCount > 0 else { return [] }
        var lower = Double.infinity
        var upper = -Double.infinity
        for value in values where value.isFinite && value > 0 {
            lower = min(lower, value)
            upper = max(upper, value)
        }
        guard lower <= upper else { return [] }
        guard lower < upper, maximumCount > 1 else { return [lower] }
        let first = ceil(log10(lower))
        let last = floor(log10(upper))
        guard first < last else { return [lower, upper] }
        let count = min(maximumCount, Int(last - first) + 1)
        let ticks = (0..<count).compactMap { index -> Double? in
            let exponent = (first + (last - first) * Double(index) / Double(count - 1)).rounded()
            let value = pow(10, exponent)
            return value.isFinite && value > 0 && value >= lower && value <= upper ? value : nil
        }
        return ticks.isEmpty ? [lower, upper] : ticks
    }

    static func label(_ value: Double) -> String {
        value.formatted(
            .number.notation(.scientific).precision(.significantDigits(1...3))
                .locale(Locale(identifier: "en_US_POSIX"))
        ).lowercased()
    }
}

/// Convert the bounded sample once on the analysis worker, never on hover.
/// Unsupported calendar ranges keep numeric coordinates and exact hover labels.
enum ScatterDates {
    static func context(_ dates: [Date]) -> String {
        guard let first = dates.min(), let last = dates.max() else { return "UTC" }
        let utc = TimeZone(secondsFromGMT: 0)!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = utc
        if calendar.isDate(first, inSameDayAs: last) {
            return first.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted, timeZone: utc)) + " · UTC"
        }
        let start = calendar.component(.year, from: first)
        let end = calendar.component(.year, from: last)
        return (start == end ? "\(start)" : "\(start)–\(end)") + " · UTC"
    }

    static func make(points: [ScatterPoint], x: Bool, kind: DataKind?) -> [Date]? {
        guard kind == .date || kind == .timestamp else { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let whole = ISO8601DateFormatter()
        whole.formatOptions = kind == .date ? [.withFullDate] : [.withInternetDateTime]
        whole.timeZone = TimeZone(secondsFromGMT: 0)
        var dates: [Date] = []
        dates.reserveCapacity(points.count)
        for point in points {
            var label = x ? point.xLabel : point.yLabel
            if kind == .timestamp {
                let offset = label.suffix(6).first
                if !label.hasSuffix("Z"), offset != "+", offset != "-" { label += "Z" }
            }
            guard let date = fractional.date(from: label) ?? whole.date(from: label) else { return nil }
            dates.append(date)
        }
        return dates
    }
}
