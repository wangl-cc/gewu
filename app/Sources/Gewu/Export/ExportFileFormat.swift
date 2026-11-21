import GewuKit
import UniformTypeIdentifiers

enum ExportFileFormat: String, CaseIterable {
    case parquet, csv, tsv, xlsx

    var title: String {
        switch self {
        case .parquet: "Parquet"
        case .csv: "CSV (Comma-separated values)"
        case .tsv: "TSV (Tab-separated values)"
        case .xlsx: "Excel Workbook (.xlsx)"
        }
    }

    var contentType: UTType {
        switch self {
        case .csv: .commaSeparatedText
        case .tsv: .tabSeparatedText
        case .parquet: UTType(importedAs: "org.apache.parquet", conformingTo: .data)
        case .xlsx: UTType("org.openxmlformats.spreadsheetml.sheet")!
        }
    }

    var fileType: FileType {
        switch self {
        case .csv: .csv
        case .tsv: .tsv
        case .parquet: .parquet
        case .xlsx: .xlsx
        }
    }
}

extension ParquetCompression {
    var title: String {
        switch self {
        case .zstd: "Zstd"
        case .snappy: "Snappy"
        case .gzip: "Gzip"
        case .brotli: "Brotli"
        case .lz4: "LZ4"
        case .uncompressed: "Uncompressed"
        }
    }
}
