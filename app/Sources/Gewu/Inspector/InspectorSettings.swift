import GewuKit
import Observation

/// Window-owned presentation choices; they survive column, page and worksheet changes.
@MainActor
@Observable
final class InspectorSettings {
    var histogramBins: UInt32 = 40
    var histogramScale: AxisScale = .linear
    var frequencyScale: AxisScale = .linear
    var scatterPointLimit: UInt32 = 2048
    var scatterXScale: AxisScale = .linear
    var scatterYScale: AxisScale = .linear
    private var expansionOverrides: [InspectorSection: Bool] = [:]

    /// The sections to show expanded: defaults, then the user's choices.
    func expandedSections(defaultingToDistribution distribution: Bool) -> Set<InspectorSection> {
        var sections: Set<InspectorSection> =
            distribution ? [.overview, .statistics, .distribution] : [.overview, .values]
        for (section, open) in expansionOverrides {
            if open { sections.insert(section) } else { sections.remove(section) }
        }
        return sections
    }

    func setExpanded(_ section: InspectorSection, _ open: Bool) {
        expansionOverrides[section] = open
    }

    /// The scale for binning a column; logarithmic only where supported.
    func binningScale(logarithmic: Bool) -> AxisScale {
        logarithmic ? histogramScale : .linear
    }
}
