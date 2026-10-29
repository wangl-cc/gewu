import UniformTypeIdentifiers

enum ExportFileFormat: String, CaseIterable {
    case csv, tsv, parquet, xlsx

    init?(contentType: UTType?) {
        guard let format = Self.allCases.first(where: { $0.contentType == contentType }) else {
            return nil
        }
        self = format
    }

    var contentType: UTType {
        switch self {
        case .csv: return .commaSeparatedText
        case .tsv: return .tabSeparatedText
        case .parquet: return UTType(filenameExtension: "parquet") ?? .data
        case .xlsx: return UTType("org.openxmlformats.spreadsheetml.sheet") ?? .spreadsheet
        }
    }

    var coreFormat: FileType {
        switch self {
        case .csv: return .csv
        case .tsv: return .tsv
        case .parquet: return .parquet
        case .xlsx: return .xlsx
        }
    }
}
