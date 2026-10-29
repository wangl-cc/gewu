import UniformTypeIdentifiers

enum ExportFileFormat: String, CaseIterable {
    case csv, tsv, parquet

    init?(contentType: UTType?) {
        guard let format = Self.allCases.first(where: { $0.contentType == contentType }) else {
            return nil
        }
        self = format
    }

    var supportsJSONConversion: Bool { self == .csv || self == .tsv }

    var contentType: UTType {
        switch self {
        case .csv: return .commaSeparatedText
        case .tsv: return .tabSeparatedText
        case .parquet: return UTType(filenameExtension: "parquet") ?? .data
        }
    }

    var coreFormat: ExportFormat {
        switch self {
        case .csv: return .csv
        case .tsv: return .tsv
        case .parquet: return .parquet
        }
    }
}
