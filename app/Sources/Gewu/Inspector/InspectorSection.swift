/// UI sections and preference keys, independent of the backend analysis API.
enum InspectorSection: Hashable {
    case overview
    case statistics
    case distribution
    case values
    case lengths
    case structure

    static func available(for capabilities: AnalysisCapabilities) -> [Self] {
        var sections: [Self] = [.overview]
        if capabilities.statistics { sections.append(.statistics) }
        if capabilities.distribution { sections.append(.distribution) }
        if capabilities.values { sections.append(.values) }
        if capabilities.lengths { sections.append(.lengths) }
        if capabilities.structure { sections.append(.structure) }
        return sections
    }

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

    // Stable preference bits, independent of presentation order.
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
