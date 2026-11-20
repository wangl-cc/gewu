import Foundation
internal import GewuBindings

/// A stable column identity within one opened table. Unaffected by hiding or
/// reordering columns.
public struct ColumnID: Hashable, Sendable, Codable {
    /// An opaque number, unique among the table's columns.
    public let raw: UInt32

    init(_ id: GewuBindings.ColumnId) { raw = id }
    var ffi: GewuBindings.ColumnId { raw }
}

/// How the app should present a column's values.
public enum ValueCategory: Sendable {
    case number, temporal, text
}

/// Which filter editors and chart axes a column supports.
public struct ColumnCapabilities: Hashable, Sendable {
    public var textMatch: Bool
    public var range: Bool
    public var exact: Bool
    public var scatter: Bool
    public var logarithmic: Bool

    public init(textMatch: Bool, range: Bool, exact: Bool, scatter: Bool, logarithmic: Bool) {
        self.textMatch = textMatch
        self.range = range
        self.exact = exact
        self.scatter = scatter
        self.logarithmic = logarithmic
    }
}

/// Which analyses a column supports, known without scanning.
public struct AnalysisCapabilities: Hashable, Sendable {
    public var statistics: Bool
    public var distribution: Bool
    public var values: Bool
    public var lengths: Bool
    public var structure: Bool

    public init(statistics: Bool, distribution: Bool, values: Bool, lengths: Bool, structure: Bool) {
        self.statistics = statistics
        self.distribution = distribution
        self.values = values
        self.lengths = lengths
        self.structure = structure
    }
}

public struct ColumnInfo: Identifiable, Sendable {
    public let id: ColumnID
    public let name: String
    public let valueCategory: ValueCategory
    /// A short type name for headers, such as "Integer".
    public let displayType: String
    /// The physical Arrow type, for tooltips and file information.
    public let technicalType: String
    public let nullable: Bool
    public let capabilities: ColumnCapabilities
    public let analyses: AnalysisCapabilities
}

public enum FileType: String, CaseIterable, Sendable {
    case csv, tsv, parquet, xlsx
}

public enum FileMetadata: Sendable {
    case parquet(
        rowGroups: UInt32, compression: [String], createdBy: String?, keyValues: [(key: String, value: String)])
    case delimited
    case xlsx(sheet: String, sheets: [String])
}

/// What Core read from a file. Fixed for the life of a ``TableSource``.
public struct TableInfo: Sendable {
    public let path: String
    public let rowCount: UInt64
    public let columns: [ColumnInfo]
    public let format: FileType
    public let fileSize: UInt64
    public let metadata: FileMetadata

    /// The selected worksheet of a workbook.
    public var worksheet: String? {
        if case .xlsx(let sheet, _) = metadata { sheet } else { nil }
    }
}

/// A nested field of a column, for the Structure analysis.
public struct AnalysisField: Hashable, Sendable {
    public let name: String
    public let displayType: String
    public let technicalType: String
    public let nullable: Bool
}

/// How to read a file.
public struct OpenOptions: Hashable, Sendable {
    /// The worksheet to open; the first one when `nil`. Only for XLSX.
    public var sheet: String?
    /// Whether the first record holds column names. Ignored for Parquet.
    public var hasHeader: Bool

    public init(sheet: String? = nil, hasHeader: Bool = true) {
        self.sheet = sheet
        self.hasHeader = hasHeader
    }
}

// MARK: - Conversion from the bindings

extension ValueCategory {
    init(_ category: GewuBindings.ValueCategory) {
        switch category {
        case .number: self = .number
        case .temporal: self = .temporal
        case .text: self = .text
        }
    }
}

extension ColumnInfo {
    init(_ info: GewuBindings.ColumnInfo) {
        id = ColumnID(info.id)
        name = info.name
        valueCategory = ValueCategory(info.valueCategory)
        displayType = info.displayType
        technicalType = info.technicalType
        nullable = info.nullable
        let capabilities = info.capabilities
        self.capabilities = ColumnCapabilities(
            textMatch: capabilities.textMatch, range: capabilities.range, exact: capabilities.exact,
            scatter: capabilities.scatter, logarithmic: capabilities.logarithmic)
        let analysis = info.analysis
        analyses = AnalysisCapabilities(
            statistics: analysis.statistics, distribution: analysis.distribution, values: analysis.values,
            lengths: analysis.lengths, structure: analysis.structure)
    }
}

extension FileType {
    init(_ type: GewuBindings.FileType) {
        switch type {
        case .csv: self = .csv
        case .tsv: self = .tsv
        case .parquet: self = .parquet
        case .xlsx: self = .xlsx
        }
    }

    var ffi: GewuBindings.FileType {
        switch self {
        case .csv: .csv
        case .tsv: .tsv
        case .parquet: .parquet
        case .xlsx: .xlsx
        }
    }
}

extension TableInfo {
    init(_ info: GewuBindings.DatasetInfo) {
        path = info.path
        rowCount = info.rowCount
        columns = info.columns.map(ColumnInfo.init)
        format = FileType(info.format)
        fileSize = info.fileSize
        metadata =
            switch info.metadata {
            case .parquet(let rowGroups, let compression, let createdBy, let keyValues):
                .parquet(
                    rowGroups: rowGroups, compression: compression, createdBy: createdBy,
                    keyValues: keyValues.map { ($0.key, $0.value) })
            case .delimited: .delimited
            case .xlsx(let sheet, let sheets): .xlsx(sheet: sheet, sheets: sheets)
            }
    }
}

extension AnalysisField {
    init(_ field: GewuBindings.AnalysisField) {
        name = field.name
        displayType = field.displayType
        technicalType = field.technicalType
        nullable = field.nullable
    }
}
