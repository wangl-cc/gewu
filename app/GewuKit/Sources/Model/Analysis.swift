import Foundation

public enum AxisScale: Hashable, Sendable, Codable {
    case linear, log10
}

/// Counts over evenly spaced bins, or over `binEdges` when they are given.
/// Logarithmic histograms place `lo`, `hi` and `binEdges` in log10 space.
public struct Histogram: Hashable, Sendable {
    public var scale: AxisScale
    public var lo: Double?
    public var hi: Double?
    /// Exact original endpoint labels, independent of the plotting space.
    public var minLabel: String?
    public var maxLabel: String?
    /// Whether bins hold whole numbers, so the upper edge is inclusive.
    public var integer: Bool
    public var counts: [UInt64]
    /// One more edge than counts for nonempty logarithmic histograms; empty for linear spacing.
    public var binEdges: [Double]
    /// Values omitted from a logarithmic histogram because they are zero or negative.
    public var nonPositiveCount: UInt64
    public var infiniteCount: UInt64

    public init(
        scale: AxisScale, lo: Double?, hi: Double?, minLabel: String?, maxLabel: String?, integer: Bool,
        counts: [UInt64], binEdges: [Double], nonPositiveCount: UInt64, infiniteCount: UInt64
    ) {
        self.scale = scale
        self.lo = lo
        self.hi = hi
        self.minLabel = minLabel
        self.maxLabel = maxLabel
        self.integer = integer
        self.counts = counts
        self.binEdges = binEdges
        self.nonPositiveCount = nonPositiveCount
        self.infiniteCount = infiniteCount
    }
}

/// Summary statistics of a numeric or temporal column. Labels keep the
/// column's native precision; show them verbatim.
public struct NumericStatistics: Hashable, Sendable {
    public var minLabel: String?
    public var maxLabel: String?
    public var medianLabel: String?
    /// The mean of ordinary numbers, for the app to round.
    public var mean: Double?
    /// The approximate mean of temporal values, rounded to their unit.
    public var meanLabel: String?
    public var stdDev: Double?
    public var nanCount: UInt64
    public var infiniteCount: UInt64

    public init(
        minLabel: String?, maxLabel: String?, medianLabel: String?, mean: Double?, meanLabel: String?,
        stdDev: Double?, nanCount: UInt64, infiniteCount: UInt64
    ) {
        self.minLabel = minLabel
        self.maxLabel = maxLabel
        self.medianLabel = medianLabel
        self.mean = mean
        self.meanLabel = meanLabel
        self.stdDev = stdDev
        self.nanCount = nanCount
        self.infiniteCount = infiniteCount
    }
}

public struct ValueFrequency: Hashable, Sendable {
    /// Display text only; not a filter value.
    public var label: String
    public var count: UInt64

    public init(label: String, count: UInt64) {
        self.label = label
        self.count = count
    }
}

/// Frequent values with an estimated distinct count. Candidates are
/// estimated and can omit values that are frequent overall.
public struct ValueSummary: Hashable, Sendable {
    public var distinctCount: UInt64
    public var values: [ValueFrequency]

    public init(distinctCount: UInt64, values: [ValueFrequency]) {
        self.distinctCount = distinctCount
        self.values = values
    }
}

public enum LengthUnit: Hashable, Sendable {
    case characters, bytes, elements
}

public struct LengthSummary: Hashable, Sendable {
    public var unit: LengthUnit
    public var emptyCount: UInt64
    public var min: UInt64?
    public var max: UInt64?
    public var mean: Double?
    public var histogram: Histogram

    public init(unit: LengthUnit, emptyCount: UInt64, min: UInt64?, max: UInt64?, mean: Double?, histogram: Histogram)
    {
        self.unit = unit
        self.emptyCount = emptyCount
        self.min = min
        self.max = max
        self.mean = mean
        self.histogram = histogram
    }
}

// MARK: - Scatter

/// How to read a scatter axis.
public enum ScatterAxisKind: Hashable, Sendable {
    case number
    /// Calendar dates; ``ScatterPoint`` carries each point's `Date` at midnight UTC.
    case date
    /// Instants; ``ScatterPoint`` carries each point's `Date`.
    case dateTime
}

public struct ScatterPoint: Hashable, Sendable {
    /// Zero-based source row, independent of sorting.
    public var sourceRow: UInt64
    /// Approximate plot coordinates. Temporal axes use seconds since 1970;
    /// use the labels for original precision.
    public var x: Double
    public var y: Double
    public var xLabel: String
    public var yLabel: String

    public init(sourceRow: UInt64, x: Double, y: Double, xLabel: String, yLabel: String) {
        self.sourceRow = sourceRow
        self.x = x
        self.y = y
        self.xLabel = xLabel
        self.yLabel = yLabel
    }

    public var xDate: Date { Date(timeIntervalSince1970: x) }
    public var yDate: Date { Date(timeIntervalSince1970: y) }
}

/// A deterministic sample of plottable pairs, chosen by source row.
public struct ScatterSample: Hashable, Sendable {
    public var xAxis: ScatterAxisKind
    public var yAxis: ScatterAxisKind
    /// Rows with finite values in both axis domains; more than `points.count` means sampled.
    public var eligibleRows: UInt64
    /// Finite pairs omitted because a logarithmic axis is zero or negative.
    public var nonPositiveRows: UInt64
    public var points: [ScatterPoint]

    public init(
        xAxis: ScatterAxisKind, yAxis: ScatterAxisKind, eligibleRows: UInt64, nonPositiveRows: UInt64,
        points: [ScatterPoint]
    ) {
        self.xAxis = xAxis
        self.yAxis = yAxis
        self.eligibleRows = eligibleRows
        self.nonPositiveRows = nonPositiveRows
        self.points = points
    }

    public var isSampled: Bool { eligibleRows > UInt64(points.count) }
}

// MARK: - Filter editors

/// One selectable value of a column, with its count in the edited rows.
public struct ValueChoice: Hashable, Sendable {
    /// Display text.
    public var label: String
    /// The exact literal to filter by.
    public var value: String
    public var count: UInt64

    public init(label: String, value: String, count: UInt64) {
        self.label = label
        self.value = value
        self.count = count
    }
}

/// The values a filter editor can offer for a column.
public struct FilterOptions: Hashable, Sendable {
    /// Every value present, or `nil` when no complete bounded list exists.
    public var values: [ValueChoice]?
    public var nullCount: UInt64

    public init(values: [ValueChoice]?, nullCount: UInt64) {
        self.values = values
        self.nullCount = nullCount
    }
}

/// An exact bin endpoint for a range filter.
public struct RangeBound: Hashable, Sendable {
    /// The exact literal to filter by.
    public var value: String
    public var label: String
    /// Plot position in the column's original units.
    public var coordinate: Double

    public init(value: String, label: String, coordinate: Double) {
        self.value = value
        self.label = label
        self.coordinate = coordinate
    }
}

public struct RangeBounds: Hashable, Sendable {
    public var min: RangeBound?
    public var max: RangeBound?

    public init(min: RangeBound?, max: RangeBound?) {
        self.min = min
        self.max = max
    }
}

/// A histogram whose bins carry exact endpoints a range filter can use.
public struct RangeHistogram: Hashable, Sendable {
    public var histogram: Histogram
    /// One entry per bin.
    public var ranges: [RangeBounds]

    public init(histogram: Histogram, ranges: [RangeBounds]) {
        self.histogram = histogram
        self.ranges = ranges
    }
}

// MARK: - Conversion from the bindings

extension AxisScale {
    init(_ scale: FfiAxisScale) {
        switch scale {
        case .linear: self = .linear
        case .log10: self = .log10
        }
    }

    var ffi: FfiAxisScale {
        switch self {
        case .linear: .linear
        case .log10: .log10
        }
    }
}

extension Histogram {
    init(_ h: FfiHistogram) {
        self.init(
            scale: AxisScale(h.scale), lo: h.lo, hi: h.hi, minLabel: h.minLabel, maxLabel: h.maxLabel,
            integer: h.integer, counts: h.counts, binEdges: h.binEdges, nonPositiveCount: h.nonPositiveCount,
            infiniteCount: h.infiniteCount)
    }
}

extension NumericStatistics {
    init(_ s: FfiNumericStatistics) {
        self.init(
            minLabel: s.minLabel, maxLabel: s.maxLabel, medianLabel: s.medianLabel, mean: s.mean,
            meanLabel: s.meanLabel, stdDev: s.stdDev, nanCount: s.nanCount, infiniteCount: s.infiniteCount)
    }
}

extension ValueSummary {
    init(_ summary: FfiValueSummary) {
        self.init(
            distinctCount: summary.distinctCount,
            values: summary.values.map { ValueFrequency(label: $0.label, count: $0.count) })
    }
}

extension LengthSummary {
    init(_ summary: FfiLengthSummary) {
        let unit: LengthUnit =
            switch summary.unit {
            case .characters: .characters
            case .bytes: .bytes
            case .elements: .elements
            }
        self.init(
            unit: unit, emptyCount: summary.emptyCount, min: summary.min, max: summary.max, mean: summary.mean,
            histogram: Histogram(summary.histogram))
    }
}

extension ScatterSample {
    init(_ result: FfiScatterResult) {
        func axis(_ kind: FfiScatterAxisKind) -> (ScatterAxisKind, Double) {
            switch kind {
            case .number: (.number, 1)
            case .date(let seconds): (.date, seconds)
            case .dateTime(let seconds): (.dateTime, seconds)
            }
        }
        let (xAxis, xSeconds) = axis(result.xAxisKind)
        let (yAxis, ySeconds) = axis(result.yAxisKind)
        self.init(
            xAxis: xAxis, yAxis: yAxis, eligibleRows: result.eligibleRows, nonPositiveRows: result.nonPositiveRows,
            points: result.points.map {
                ScatterPoint(
                    sourceRow: $0.sourceRow, x: $0.x * xSeconds, y: $0.y * ySeconds, xLabel: $0.xLabel,
                    yLabel: $0.yLabel)
            })
    }
}

extension FilterOptions {
    init(_ choices: FfiFilterChoices) {
        self.init(
            values: choices.values?.map { ValueChoice(label: $0.label, value: $0.value, count: $0.count) },
            nullCount: choices.nullCount)
    }
}

extension RangeHistogram {
    init(_ h: FfiRangeHistogram) {
        func bound(_ b: FfiRangeBound?) -> RangeBound? {
            b.map { RangeBound(value: $0.value, label: $0.label, coordinate: $0.coordinate) }
        }
        self.init(
            histogram: Histogram(h.histogram),
            ranges: h.ranges.map { RangeBounds(min: bound($0.min), max: bound($0.max)) })
    }
}
