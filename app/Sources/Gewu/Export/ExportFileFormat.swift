import UniformTypeIdentifiers

enum ExportFileFormat: String, CaseIterable {
    case parquet, csv, tsv, xlsx

    var title: String {
        switch self {
        case .parquet: return "Parquet"
        case .csv: return "CSV (Comma-separated values)"
        case .tsv: return "TSV (Tab-separated values)"
        case .xlsx: return "Excel Workbook (.xlsx)"
        }
    }

    var contentType: UTType {
        switch self {
        case .csv: return .commaSeparatedText
        case .tsv: return .tabSeparatedText
        case .parquet: return UTType(importedAs: "io.github.wangl-cc.gewu.parquet", conformingTo: .data)
        case .xlsx: return UTType(importedAs: "org.openxmlformats.spreadsheetml.sheet", conformingTo: .spreadsheet)
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

enum ExportCompression: String, CaseIterable {
    case zstd, snappy, gzip, brotli, lz4, uncompressed

    var title: String {
        switch self {
        case .zstd: return "Zstd"
        case .snappy: return "Snappy"
        case .gzip: return "Gzip"
        case .brotli: return "Brotli"
        case .lz4: return "LZ4"
        case .uncompressed: return "Uncompressed"
        }
    }

    var coreCompression: ParquetCompression {
        switch self {
        case .zstd: return .zstd
        case .snappy: return .snappy
        case .gzip: return .gzip
        case .brotli: return .brotli
        case .lz4: return .lz4
        case .uncompressed: return .uncompressed
        }
    }
}
