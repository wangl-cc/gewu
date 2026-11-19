import Foundation
import Observation

/// Owns the current column's analysis request. The Viewer retains completed results;
/// closing or replacing this presentation cancels its pending computations.
@MainActor
@Observable
final class InspectorModel {
    private(set) var sections: [InspectorSection] = []
    private(set) var nullCount: UInt64?
    private(set) var rowCount: UInt64 = 0
    private(set) var statistics: NumericStatistics?
    private(set) var distribution: Histogram?
    private(set) var values: ValueSummary?
    private(set) var lengths: LengthSummary?
    private(set) var structure: [AnalysisField]?
    private(set) var errors: [InspectorSection: String] = [:]
    private(set) var loading: InspectorSection?
    private(set) var error: String?

    private struct Context: Equatable {
        let filtered: GewuFilteredView
        let column: ColumnId
        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.filtered === rhs.filtered && lhs.column == rhs.column
        }
    }

    private var context: Context?
    @ObservationIgnored private var request: UInt64 = 0
    @ObservationIgnored private var work: Task<Void, Never>?
    @ObservationIgnored private var histogramBins: UInt32 = 40
    @ObservationIgnored private var histogramScale: AxisScale = .linear

    func matches(filtered: GewuFilteredView, column: ColumnId) -> Bool {
        context == Context(filtered: filtered, column: column)
    }

    /// Stop this panel's work without affecting other consumers.
    func cancel() {
        request &+= 1
        work?.cancel()
        work = nil
        loading = nil
    }

    func load(
        column: ColumnInfo, dataset: GewuDataset, viewer: Viewer, expanded: Int,
        bins: UInt32 = 40, scale: AxisScale = .linear
    ) async {
        guard !Task.isCancelled else { return }
        let filtered = viewer.filtered
        cancel()
        let ticket = request
        let task = Task {
            await performLoad(
                column: column, dataset: dataset, viewer: viewer, filtered: filtered,
                expanded: expanded, bins: bins, scale: scale, ticket: ticket)
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
        column: ColumnInfo, dataset: GewuDataset, viewer: Viewer, filtered: GewuFilteredView,
        expanded: Int, bins: UInt32, scale: AxisScale, ticket: UInt64
    ) async {
        guard !Task.isCancelled, request == ticket, viewer.filtered === filtered else { return }
        defer {
            if request == ticket { loading = nil }
        }
        let next = Context(filtered: filtered, column: column.id)
        if context != next {
            context = next
            rowCount = filtered.rowCount()
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
        sections = InspectorSection.available(for: column.analysis)
        error = nil
        for section in sections where expanded & section.mask != 0 && !hasResult(section) {
            guard !Task.isCancelled, request == ticket, viewer.filtered === filtered else { return }
            loading = section
            errors[section] = nil
            do {
                switch section {
                case .overview:
                    try await load(\.nullCount, ticket: ticket, context: next, viewer: viewer) {
                        try await viewer.countNulls(column.id)
                    }
                case .statistics:
                    try await load(\.statistics, ticket: ticket, context: next, viewer: viewer) {
                        try await viewer.statistics(column.id)
                    }
                case .distribution:
                    try await load(\.distribution, ticket: ticket, context: next, viewer: viewer) {
                        try await viewer.distribution(column.id, bins: bins, scale: scale)
                    }
                case .values:
                    try await load(\.values, ticket: ticket, context: next, viewer: viewer) {
                        try await viewer.values(column.id)
                    }
                case .lengths:
                    try await load(\.lengths, ticket: ticket, context: next, viewer: viewer) {
                        try await viewer.lengths(column.id, bins: bins)
                    }
                case .structure:
                    structure = try dataset.columnStructure(column: column.id)
                }
            } catch {
                guard !Task.isCancelled, request == ticket else { return }
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

    private func load<T>(
        _ destination: ReferenceWritableKeyPath<InspectorModel, T?>,
        ticket: UInt64,
        context expected: Context,
        viewer: Viewer,
        operation: () async throws -> T
    ) async throws {
        try Task.checkCancellation()
        guard request == ticket, context == expected, viewer.filtered === expected.filtered else {
            throw CancellationError()
        }
        let result = try await operation()
        try Task.checkCancellation()
        guard request == ticket, context == expected, viewer.filtered === expected.filtered else {
            throw CancellationError()
        }
        self[keyPath: destination] = result
    }
}
