import Foundation
import GewuKit
import Observation

/// The analysis results shown for one column of one row set. The row set
/// computes and caches them; this model only presents the current request's
/// sections, loading expanded ones in order. The owning view runs ``load(_:)``
/// in `.task(id:)`, so a new request cancels the previous one.
@MainActor
@Observable
final class InspectorModel {
    struct Request: Equatable {
        let rows: RowSet
        let column: ColumnID
        let expanded: Set<InspectorSection>
        let bins: UInt32
        let scale: AxisScale
        /// Changed by Retry, to run the same request again.
        let attempt: Int
    }

    private(set) var rows: RowSet?
    private(set) var column: ColumnID?
    private(set) var sections: [InspectorSection] = []
    private(set) var nullCount: UInt64?
    private(set) var statistics: NumericStatistics?
    private(set) var distribution: Histogram?
    private(set) var values: ValueSummary?
    private(set) var lengths: LengthSummary?
    private(set) var structure: [AnalysisField]?
    private(set) var errors: [InspectorSection: String] = [:]
    private(set) var loading: InspectorSection?
    @ObservationIgnored private var binning: (bins: UInt32, scale: AxisScale)?

    func shows(_ rows: RowSet, column: ColumnID) -> Bool {
        self.rows === rows && self.column == column
    }

    func load(_ request: Request) async {
        guard let info = request.rows.source.column(request.column) else { return }
        if !shows(request.rows, column: request.column) {
            rows = request.rows
            column = request.column
            nullCount = nil
            statistics = nil
            distribution = nil
            values = nil
            lengths = nil
            structure = nil
            errors = [:]
            binning = nil
        }
        if binning?.bins != request.bins {
            distribution = nil
            lengths = nil
        } else if binning?.scale != request.scale {
            distribution = nil
        }
        binning = (request.bins, request.scale)
        errors = [:]
        sections = InspectorSection.available(for: info.analyses)
        defer { loading = nil }
        for section in sections where request.expanded.contains(section) && !hasResult(section) {
            guard !Task.isCancelled else { return }
            loading = section
            do {
                try await fetch(section, request)
            } catch {
                guard !Task.isCancelled, !(error is CancellationError) else { return }
                errors[section] = ErrorPresentation(error).text
            }
        }
    }

    private func fetch(_ section: InspectorSection, _ request: Request) async throws {
        let rows = request.rows
        let column = request.column
        switch section {
        case .overview:
            let value = try await rows.nullCount(column)
            try Task.checkCancellation()
            nullCount = value
        case .statistics:
            let value = try await rows.statistics(column)
            try Task.checkCancellation()
            statistics = value
        case .distribution:
            let value = try await rows.distribution(column, bins: request.bins, scale: request.scale)
            try Task.checkCancellation()
            distribution = value
        case .values:
            let value = try await rows.values(column)
            try Task.checkCancellation()
            values = value
        case .lengths:
            let value = try await rows.lengths(column, bins: request.bins)
            try Task.checkCancellation()
            lengths = value
        case .structure:
            structure = try rows.source.structure(of: column)
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
}
