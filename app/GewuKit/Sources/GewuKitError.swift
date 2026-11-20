import Foundation
internal import GewuBindings

/// A failure reported by the data engine.
///
/// ``kind`` says what went wrong, with the values involved, so callers can
/// react to specific failures and describe them in their own words;
/// ``errorDescription`` is such a description. ``diagnostic`` is the
/// engine's own English text, for logs. When the kind alone cannot say what
/// happened, such as for a parser's or the file system's message,
/// ``failureReason`` returns the diagnostic as the details to show. Cancellation is never a
/// `GewuKitError`: GewuKit reports it as `CancellationError`, whether Swift or
/// Rust cancelled the work.
public struct GewuKitError: LocalizedError, Equatable, Sendable {
    public let kind: Kind
    public let diagnostic: String

    public init(_ kind: Kind, diagnostic: String) {
        self.kind = kind
        self.diagnostic = diagnostic
    }

    public enum Kind: Equatable, Sendable {
        /// The file system refused an operation on a file.
        case fileAccess(FileOperation)
        /// The file's content or format is not supported.
        case file(FileProblem)
        /// The table exceeds a size limit.
        case limit(SizeLimit)
        /// A filter value is invalid.
        case filter(FilterProblem)
        /// The search text cannot be matched.
        case search
        /// The rows do not support the requested analysis.
        case analysis(AnalysisProblem)
        /// An export or copy could not be written.
        case export(ExportProblem)
        /// The app called the engine incorrectly. Indicates a defect.
        case internalFailure
    }

    public enum FileOperation: Sendable {
        case resolvePath, readMetadata, open, read, seek, createTemporaryFile, sync, commit
    }

    public enum FileProblem: Equatable, Sendable {
        case unknownFormat
        case formatDisabled(FileType)
        case decode(FileType)
        case noWorksheets
        case sheetNotFound(String)
        /// A cell holds an Excel error value, such as `#DIV/0!`; `row` and `column` count from 1.
        case errorCell(sheet: String, row: UInt32, column: UInt32, value: String)
        case unsupportedColumnType(column: String, dataType: String)
    }

    public enum SizeLimit: Equatable, Sendable {
        case datasetRows(actual: UInt64, limit: UInt64)
        case worksheetRows(actual: UInt64, limit: UInt64)
        case worksheetColumns(actual: UInt32, limit: UInt32)
    }

    public enum FilterProblem: Equatable, Sendable {
        case invalidValue(String, expectedType: String, reason: ValueProblem)
        case reversedRange(min: String, max: String)
        case unmatchableText
    }

    public enum ValueProblem: Sendable {
        case invalidSyntax, outOfRange, notANumber
    }

    public enum AnalysisProblem: Equatable, Sendable {
        case noFiniteValues
        case unrepresentableDomain(dataType: String)
        case valueOutOfRange(Analysis, dataType: String)
        case unsupportedType(Analysis, dataType: String)
    }

    public enum Analysis: Sendable {
        case statistics, distribution, values, lengths, structure, scatter, filterDistribution
    }

    public enum ExportProblem: Equatable, Sendable {
        case sourceIsDestination
        case nestedColumns(FileType, [ColumnID])
        case encode(FileType)
        /// The output would exceed this many bytes.
        case sizeLimit(UInt64)
    }

    public var errorDescription: String? { kind.description }

    public var failureReason: String? {
        switch kind {
        case .fileAccess, .file(.decode), .export(.encode), .internalFailure: diagnostic
        default: nil
        }
    }
}

extension GewuKitError.Kind: CustomStringConvertible {
    public var description: String {
        switch self {
        case .fileAccess(let operation):
            switch operation {
            case .resolvePath, .readMetadata, .open: "The file could not be opened."
            case .read, .seek: "The file could not be read."
            case .createTemporaryFile, .sync, .commit: "The file could not be saved."
            }
        case .file(let problem):
            switch problem {
            case .unknownFormat: "The file is not a Parquet, CSV, TSV or XLSX file."
            case .formatDisabled(let format): "This build cannot read \(format.displayName) files."
            case .decode(let format): "The file could not be read as \(format.displayName)."
            case .noWorksheets: "The workbook has no worksheets."
            case .sheetNotFound(let sheet): "The workbook has no worksheet named “\(sheet)”."
            case .errorCell(let sheet, let row, let column, let value):
                "Worksheet “\(sheet)” has the error value \(value) at row \(row), column \(column)."
            case .unsupportedColumnType(let column, let dataType):
                "Column “\(column)” has a type Gewu cannot show (\(dataType))."
            }
        case .limit(let limit):
            switch limit {
            case .datasetRows(let actual, let limit):
                "The table has \(actual) rows; Gewu supports at most \(limit)."
            case .worksheetRows(let actual, let limit):
                "A worksheet holds at most \(limit) rows, not \(actual)."
            case .worksheetColumns(let actual, let limit):
                "A worksheet holds at most \(limit) columns, not \(actual)."
            }
        case .filter(let problem):
            switch problem {
            case .invalidValue(let value, let type, .invalidSyntax): "“\(value)” is not a valid \(type) value."
            case .invalidValue(let value, let type, .outOfRange): "“\(value)” is out of range for \(type)."
            case .invalidValue(let value, _, .notANumber): "“\(value)” cannot be used as a bound."
            case .reversedRange(let min, let max): "The minimum \(min) is greater than the maximum \(max)."
            case .unmatchableText: "The filter text is too complex to match."
            }
        case .search: "The search text is too complex to match."
        case .analysis(let problem):
            switch problem {
            case .noFiniteValues: "There are no finite values to analyze."
            case .unrepresentableDomain(let type): "These \(type) values cannot be placed on an axis."
            case .valueOutOfRange(let analysis, let type):
                "\(analysis.displayName): some \(type) values are out of range."
            case .unsupportedType(let analysis, let type):
                "\(analysis.displayName) is not available for \(type) columns."
            }
        case .export(let problem):
            switch problem {
            case .sourceIsDestination: "The export would replace the file it reads from."
            case .nestedColumns(let format, _): "\(format.displayName) cannot hold nested columns."
            case .encode(let format): "The data could not be written as \(format.displayName)."
            case .sizeLimit(let limit):
                "The result is larger than \(Int64(clamping: limit).formatted(.byteCount(style: .memory)))."
            }
        case .internalFailure: "Gewu encountered an internal error."
        }
    }
}

extension GewuKitError.Analysis {
    var displayName: String {
        switch self {
        case .statistics: "Statistics"
        case .distribution: "Distribution"
        case .values: "Values"
        case .lengths: "Lengths"
        case .structure: "Structure"
        case .scatter: "Scatter"
        case .filterDistribution: "Range filter"
        }
    }
}

// MARK: - Conversion from the bindings

extension GewuBindings.GewuError {
    var translated: any Error {
        switch self {
        case .Io(let operation, let message): GewuKitError(.fileAccess(.init(operation)), diagnostic: message)
        case .File(let kind, let message): GewuKitError(.file(.init(kind)), diagnostic: message)
        case .Limit(let kind, let message): GewuKitError(.limit(.init(kind)), diagnostic: message)
        case .Filter(let kind, let message): GewuKitError(.filter(.init(kind)), diagnostic: message)
        case .Find(.queryCompilationFailed, let message): GewuKitError(.search, diagnostic: message)
        case .Analysis(let kind, let message): GewuKitError(.analysis(.init(kind)), diagnostic: message)
        case .Export(let kind, let message): GewuKitError(.export(.init(kind)), diagnostic: message)
        case .Internal(let message): GewuKitError(.internalFailure, diagnostic: message)
        case .Cancelled: CancellationError()
        }
    }
}

extension GewuKitError.FileOperation {
    init(_ operation: GewuBindings.IoOperation) {
        self =
            switch operation {
            case .canonicalize: .resolvePath
            case .readMetadata: .readMetadata
            case .open: .open
            case .read: .read
            case .seek: .seek
            case .createTemporaryFile: .createTemporaryFile
            case .sync: .sync
            case .commit: .commit
            }
    }
}

extension GewuKitError.FileProblem {
    init(_ kind: GewuBindings.FileErrorKind) {
        self =
            switch kind {
            case .unknownFormat: .unknownFormat
            case .formatDisabled(let format): .formatDisabled(FileType(format))
            case .decode(let format): .decode(FileType(format))
            case .noWorksheets: .noWorksheets
            case .sheetNotFound(let sheet): .sheetNotFound(sheet)
            case .errorCell(let sheet, let row, let column, let error):
                .errorCell(sheet: sheet, row: row, column: column, value: error)
            case .unsupportedColumnType(let column, let dataType):
                .unsupportedColumnType(column: column, dataType: dataType)
            }
    }
}

extension GewuKitError.SizeLimit {
    init(_ kind: GewuBindings.LimitErrorKind) {
        self =
            switch kind {
            case .datasetRows(let actual, let limit): .datasetRows(actual: actual, limit: limit)
            case .xlsxRows(let actual, let limit): .worksheetRows(actual: actual, limit: limit)
            case .xlsxColumns(let actual, let limit): .worksheetColumns(actual: actual, limit: limit)
            }
    }
}

extension GewuKitError.FilterProblem {
    init(_ kind: GewuBindings.FilterErrorKind) {
        self =
            switch kind {
            case .invalidValue(let value, let expectedType, let reason):
                .invalidValue(value, expectedType: expectedType, reason: GewuKitError.ValueProblem(reason))
            case .reversedRange(let min, let max): .reversedRange(min: min, max: max)
            case .patternCompilationFailed: .unmatchableText
            }
    }
}

extension GewuKitError.ValueProblem {
    init(_ reason: GewuBindings.ValueParseError) {
        self =
            switch reason {
            case .invalidSyntax: .invalidSyntax
            case .outOfRange: .outOfRange
            case .naNNotAllowed: .notANumber
            }
    }
}

extension GewuKitError.AnalysisProblem {
    init(_ kind: GewuBindings.AnalysisErrorKind) {
        self =
            switch kind {
            case .noFiniteValues: .noFiniteValues
            case .unrepresentableDomain(let dataType): .unrepresentableDomain(dataType: dataType)
            case .valueOutOfRange(let operation, let dataType):
                .valueOutOfRange(GewuKitError.Analysis(operation), dataType: dataType)
            case .unsupportedDataType(let operation, let dataType):
                .unsupportedType(GewuKitError.Analysis(operation), dataType: dataType)
            }
    }
}

extension GewuKitError.Analysis {
    init(_ operation: GewuBindings.AnalysisOperation) {
        self =
            switch operation {
            case .statistics: .statistics
            case .distribution: .distribution
            case .values: .values
            case .lengths: .lengths
            case .structure: .structure
            case .scatter: .scatter
            case .filterDistribution: .filterDistribution
            }
    }
}

extension GewuKitError.ExportProblem {
    init(_ kind: GewuBindings.ExportErrorKind) {
        self =
            switch kind {
            case .sourceIsDestination: .sourceIsDestination
            case .nestedColumns(let format, let columns): .nestedColumns(FileType(format), columns.map(ColumnID.init))
            case .encode(let format): .encode(FileType(format))
            case .sizeLimit(let limit): .sizeLimit(limit)
            }
    }
}
