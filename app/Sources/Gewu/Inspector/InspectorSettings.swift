import SwiftUI

/// Window-owned presentation choices survive column and Inspector page changes.
@MainActor
final class InspectorSettings: ObservableObject {
    @Published var histogramBins: UInt32 = 40
    @Published var histogramScale: AxisScale = .linear
    @Published var frequencyScale: AxisScale = .linear
    @Published private var expansionOverrides: [InspectorSection: Bool] = [:]

    func expanded(distribution: Bool) -> Int {
        var mask = distribution ? 7 : 9
        for (section, open) in expansionOverrides {
            if open { mask |= section.mask } else { mask &= ~section.mask }
        }
        return mask
    }

    func setExpanded(_ section: InspectorSection, _ open: Bool) {
        expansionOverrides[section] = open
    }

    func binningScale(logarithmic: Bool) -> AxisScale {
        logarithmic ? histogramScale : .linear
    }
}
