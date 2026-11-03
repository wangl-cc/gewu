import SwiftUI

/// Owns only the current column's analysis. Its Rust cancellation handle never
/// cancels document filtering or export.
@MainActor
final class InspectorModel: ObservableObject {
    @Published private(set) var sections: [InspectorSection] = []
    @Published private(set) var nullCount: UInt64?
    private(set) var rowCount: UInt64 = 0
    @Published private(set) var statistics: NumericStatistics?
    @Published private(set) var distribution: Histogram?
    @Published private(set) var values: ValueSummary?
    @Published private(set) var lengths: LengthSummary?
    @Published private(set) var structure: [AnalysisField]?
    @Published private(set) var errors: [InspectorSection: String] = [:]
    @Published private(set) var loading: InspectorSection?
    @Published private(set) var error: String?

    private struct Context: Equatable {
        let view: GewuView
        let column: UInt32
        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.view === rhs.view && lhs.column == rhs.column
        }
    }
    private var context: Context?
    private var request: UInt64 = 0
    private var cancellation: Cancellation?
    private var histogramBins: UInt32 = 40
    private var histogramScale: AxisScale = .linear

    func matches(view: GewuView, column: UInt32) -> Bool {
        context == Context(view: view, column: column)
    }

    func cancel() {
        request &+= 1
        cancellation?.cancel()
        cancellation = nil
        loading = nil
    }

    func load(
        dataset: GewuDataset, view: GewuView, column: UInt32, expanded: Int, bins: UInt32 = 40,
        scale: AxisScale = .linear
    ) async {
        guard !Task.isCancelled else { return }
        cancel()
        let ticket = request
        defer {
            if request == ticket {
                cancellation = nil
                loading = nil
            }
        }
        let next = Context(view: view, column: column)
        if context != next {
            context = next
            self.rowCount = view.rowCount()
            sections = []
            nullCount = nil
            statistics = nil
            distribution = nil
            values = nil
            lengths = nil
            structure = nil
            errors = [:]
            error = nil
        }
        if histogramBins != bins {
            histogramBins = bins
            distribution = nil
            lengths = nil
            errors[.distribution] = nil
            errors[.lengths] = nil
        }
        if histogramScale != scale {
            histogramScale = scale
            distribution = nil
            errors[.distribution] = nil
        }
        do {
            sections = InspectorSection.available(for: try dataset.analysisCapabilities(column: column))
            error = nil
        } catch {
            self.error = error.userMessage
            return
        }
        for section in sections where expanded & section.mask != 0 && !hasResult(section) {
            guard !Task.isCancelled, request == ticket else { return }
            loading = section
            errors[section] = nil
            do {
                switch section {
                case .overview:
                    try await load(\.nullCount, ticket: ticket, context: next) { cancellation in
                        try view.countNulls(column: column, cancellation: cancellation)
                    }
                case .statistics:
                    try await load(\.statistics, ticket: ticket, context: next) { cancellation in
                        try view.analyzeStatistics(column: column, cancellation: cancellation)
                    }
                case .distribution:
                    try await load(\.distribution, ticket: ticket, context: next) { cancellation in
                        try view.analyzeDistribution(
                            column: column, bins: bins, scale: scale, cancellation: cancellation)
                    }
                case .values:
                    try await load(\.values, ticket: ticket, context: next) { cancellation in
                        try view.analyzeValues(column: column, cancellation: cancellation)
                    }
                case .lengths:
                    try await load(\.lengths, ticket: ticket, context: next) { cancellation in
                        try view.analyzeLengths(
                            column: column, bins: bins, cancellation: cancellation)
                    }
                case .structure:
                    structure = try dataset.columnStructure(column: column)
                }
            } catch {
                guard !Task.isCancelled, request == ticket else { return }
                if case GewuError.Cancelled = error { break }
                if error is CancellationError { break }
                errors[section] = error.userMessage
            }
        }
    }

    private func hasResult(_ section: InspectorSection) -> Bool {
        switch section {
        case .overview: nullCount != nil
        case .statistics: statistics != nil
        case .distribution: distribution != nil
        case .values: values != nil
        case .lengths: lengths != nil
        case .structure: structure != nil
        }
    }

    private func load<T: Sendable>(
        _ destination: ReferenceWritableKeyPath<InspectorModel, T?>,
        ticket: UInt64,
        context expected: Context,
        operation: @escaping @Sendable (Cancellation) throws -> T
    ) async throws {
        try Task.checkCancellation()
        guard request == ticket, context == expected else { throw CancellationError() }
        let signal = Cancellation()
        cancellation = signal
        defer {
            if request == ticket { cancellation = nil }
        }
        let result = try await runCancellable(cancellation: signal) { try operation(signal) }
        try Task.checkCancellation()
        guard request == ticket, context == expected else { throw CancellationError() }
        self[keyPath: destination] = result
    }
}
