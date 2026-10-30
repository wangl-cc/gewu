import SwiftUI

/// Owns only the current column's analysis. Its Rust cancellation handle never
/// cancels document filtering or export.
@MainActor
final class InspectorModel: ObservableObject {
    @Published private(set) var sections: [AnalysisSection] = []
    @Published private(set) var results: [AnalysisSection: AnalysisData] = [:]
    @Published private(set) var errors: [AnalysisSection: String] = [:]
    @Published private(set) var loading: AnalysisSection?
    @Published private(set) var error: String?

    private struct Context: Equatable {
        let document: ObjectIdentifier
        let version: UInt64
        let column: UInt32
    }
    private var context: Context?
    private var request: UInt64 = 0
    private var job: ColumnAnalysis?
    private var histogramBins: UInt32 = 40
    private var histogramScale: AxisScale = .linear

    func matches(document: GewuDocument, version: UInt64, column: UInt32) -> Bool {
        context == Context(document: ObjectIdentifier(document), version: version, column: column)
    }

    func cancel() {
        request &+= 1
        job?.cancel()
        job = nil
        loading = nil
    }

    func load(
        document: GewuDocument, version: UInt64, column: UInt32, expanded: Int, bins: UInt32 = 40,
        scale: AxisScale = .linear
    ) async {
        guard !Task.isCancelled else { return }
        cancel()
        let ticket = request
        let next = Context(document: ObjectIdentifier(document), version: version, column: column)
        if context != next {
            context = next
            sections = []
            results = [:]
            errors = [:]
            error = nil
        }
        if histogramBins != bins {
            histogramBins = bins
            for section in [AnalysisSection.distribution, .lengths] {
                results[section] = nil
                errors[section] = nil
            }
        }
        if histogramScale != scale {
            histogramScale = scale
            results[.distribution] = nil
            errors[.distribution] = nil
        }
        do {
            sections = try document.analysisCapabilities(column: column)
            error = nil
        } catch {
            self.error = error.userMessage
            return
        }
        for section in sections where expanded & section.mask != 0 && results[section] == nil {
            guard !Task.isCancelled, request == ticket else { return }
            loading = section
            errors[section] = nil
            do {
                let work = try document.prepareAnalysisWithBins(
                    version: version, column: column, section: section, bins: bins,
                    scale: section == .distribution ? scale : .linear)
                job = work
                let result = try await withTaskCancellationHandler {
                    try await Task.detached(priority: .userInitiated) { try work.run() }.value
                } onCancel: {
                    work.cancel()
                }
                guard !Task.isCancelled, request == ticket,
                    result.version == version, result.column == column
                else { return }
                results[section] = result.data
            } catch {
                guard !Task.isCancelled, request == ticket else { return }
                if case GewuError.StaleVersion = error { break }
                if case GewuError.Cancelled = error { break }
                errors[section] = error.userMessage
            }
            job = nil
        }
        guard request == ticket else { return }
        job = nil
        loading = nil
    }
}

extension AnalysisSection {
    var title: String {
        switch self {
        case .overview: "Overview"
        case .statistics: "Statistics"
        case .distribution: "Distribution"
        case .values: "Values"
        case .lengths: "Lengths"
        case .structure: "Structure"
        }
    }

    // Stable preference bits, independent of the order supplied by Core.
    var mask: Int {
        switch self {
        case .overview: 1
        case .statistics: 2
        case .distribution: 4
        case .values: 8
        case .lengths: 16
        case .structure: 32
        }
    }
}
