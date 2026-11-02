import SwiftUI

/// Owns only the current column's analysis. Its Rust cancellation handle never
/// cancels document filtering or export.
@MainActor
final class InspectorModel: ObservableObject {
    @Published private(set) var sections: [InspectorSection] = []
    @Published private(set) var overview: ColumnOverview?
    @Published private(set) var statistics: NumericStatistics?
    @Published private(set) var distribution: Histogram?
    @Published private(set) var values: ValueSummary?
    @Published private(set) var lengths: LengthSummary?
    @Published private(set) var structure: [AnalysisField]?
    @Published private(set) var errors: [InspectorSection: String] = [:]
    @Published private(set) var loading: InspectorSection?
    @Published private(set) var error: String?

    private struct Context: Equatable {
        let document: ObjectIdentifier
        let version: UInt64
        let column: UInt32
    }
    private var context: Context?
    private var request: UInt64 = 0
    private var cancellation: Cancellation?
    private var histogramBins: UInt32 = 40
    private var histogramScale: AxisScale = .linear

    func matches(document: GewuSession, version: UInt64, column: UInt32) -> Bool {
        context == Context(document: ObjectIdentifier(document), version: version, column: column)
    }

    func cancel() {
        request &+= 1
        cancellation?.cancel()
        cancellation = nil
        loading = nil
    }

    func load(
        document: GewuSession, version: UInt64, column: UInt32, expanded: Int, bins: UInt32 = 40,
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
        let next = Context(document: ObjectIdentifier(document), version: version, column: column)
        if context != next {
            context = next
            sections = []
            overview = nil
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
            sections = InspectorSection.available(for: try document.analysisCapabilities(column: column))
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
                    try await load(\.overview, ticket: ticket, context: next) { cancellation in
                        try document.analyzeOverview(version: version, column: column, cancellation: cancellation)
                    }
                case .statistics:
                    try await load(\.statistics, ticket: ticket, context: next) { cancellation in
                        try document.analyzeStatistics(version: version, column: column, cancellation: cancellation)
                    }
                case .distribution:
                    try await load(\.distribution, ticket: ticket, context: next) { cancellation in
                        try document.analyzeDistribution(
                            version: version, column: column, bins: bins, scale: scale, cancellation: cancellation)
                    }
                case .values:
                    try await load(\.values, ticket: ticket, context: next) { cancellation in
                        try document.analyzeValues(version: version, column: column, cancellation: cancellation)
                    }
                case .lengths:
                    try await load(\.lengths, ticket: ticket, context: next) { cancellation in
                        try document.analyzeLengths(
                            version: version, column: column, bins: bins, cancellation: cancellation)
                    }
                case .structure:
                    structure = try document.columnStructure(column: column)
                }
            } catch {
                guard !Task.isCancelled, request == ticket else { return }
                if case GewuError.StaleVersion = error { break }
                if case GewuError.Cancelled = error { break }
                if error is CancellationError { break }
                errors[section] = error.userMessage
            }
        }
    }

    private func hasResult(_ section: InspectorSection) -> Bool {
        switch section {
        case .overview: overview != nil
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
        let result = try await runAnalysis(cancellation: signal) { try operation(signal) }
        try Task.checkCancellation()
        guard request == ticket, context == expected else { throw CancellationError() }
        self[keyPath: destination] = result
    }
}

/// Run a synchronous scan off the UI thread and forward the owning Task's cancellation.
func runAnalysis<T: Sendable>(
    cancellation: Cancellation,
    operation: @escaping @Sendable () throws -> T
) async throws -> T {
    try await withTaskCancellationHandler {
        try Task.checkCancellation()
        let result = try await Task.detached(priority: .userInitiated) {
            try operation()
        }.value
        try Task.checkCancellation()
        return result
    } onCancel: {
        cancellation.cancel()
    }
}
