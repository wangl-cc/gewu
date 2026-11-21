import Foundation
internal import GewuBindings

/// A failure reported by the data engine.
///
/// ``kind`` says what went wrong, with the values involved, in the engine's
/// own vocabulary, so callers can react to specific failures and describe
/// them in their own words. ``diagnostic`` is the engine's English text, for
/// logs and for details that the kind alone cannot carry, such as a parser's
/// or the file system's message. GewuKit has no user-facing wording.
/// Cancellation is never a `GewuKitError`: GewuKit reports it as
/// `CancellationError`, whether Swift or Rust cancelled the work.
public struct GewuKitError: Error, Equatable, Sendable, CustomStringConvertible {
    public let kind: Kind
    public let diagnostic: String

    public init(_ kind: Kind, diagnostic: String) {
        self.kind = kind
        self.diagnostic = diagnostic
    }

    public var description: String { diagnostic }

    public enum Kind: Equatable, Sendable {
        /// The file system refused an operation.
        case io(IoOperation)
        /// The file's content or format is not supported.
        case file(FileErrorKind)
        /// The table exceeds a size limit.
        case limit(LimitErrorKind)
        /// A filter value is invalid.
        case filter(FilterErrorKind)
        /// The Find query cannot be matched.
        case find(FindErrorKind)
        /// The rows do not support the requested analysis.
        case analysis(AnalysisErrorKind)
        /// An export or copy could not be written.
        case export(ExportErrorKind)
        /// The app called the engine incorrectly. Indicates a defect.
        case `internal`
    }

    public enum IoOperation: Sendable {
        case canonicalize, readMetadata, open, read, seek, createTemporaryFile, sync, commit
    }

    public enum FileErrorKind: Equatable, Sendable {
        case unknownFormat
        case formatDisabled(FileType)
        case decode(FileType)
        case noWorksheets
        case sheetNotFound(String)
        /// A cell holds an Excel error value, such as `#DIV/0!`; `row` and `column` count from 1.
        case errorCell(sheet: String, row: UInt32, column: UInt32, error: String)
        case unsupportedColumnType(column: String, dataType: String)
    }

    public enum LimitErrorKind: Equatable, Sendable {
        case datasetRows(actual: UInt64, limit: UInt64)
        case xlsxRows(actual: UInt64, limit: UInt64)
        case xlsxColumns(actual: UInt32, limit: UInt32)
    }

    public enum FilterErrorKind: Equatable, Sendable {
        case invalidValue(value: String, expectedType: String, reason: ValueParseError)
        case reversedRange(min: String, max: String)
        case patternCompilationFailed
    }

    public enum ValueParseError: Sendable {
        case invalidSyntax, outOfRange, nanNotAllowed
    }

    public enum FindErrorKind: Sendable {
        case queryCompilationFailed
    }

    public enum AnalysisErrorKind: Equatable, Sendable {
        case noFiniteValues
        case unrepresentableDomain(dataType: String)
        case valueOutOfRange(AnalysisOperation, dataType: String)
        case unsupportedDataType(AnalysisOperation, dataType: String)
    }

    public enum AnalysisOperation: Sendable {
        case statistics, distribution, values, lengths, structure, scatter, filterDistribution
    }

    public enum ExportErrorKind: Equatable, Sendable {
        case sourceIsDestination(sourcePath: String, destination: String)
        case nestedColumns(FileType, [ColumnID])
        case encode(FileType)
        /// The output would exceed this many bytes.
        case sizeLimit(UInt64)
    }
}

// MARK: - Conversion from the bindings

extension GewuBindings.GewuError {
    var translated: any Error {
        switch self {
        case .Io(let operation, let message): GewuKitError(.io(.init(operation)), diagnostic: message)
        case .File(let kind, let message): GewuKitError(.file(.init(kind)), diagnostic: message)
        case .Limit(let kind, let message): GewuKitError(.limit(.init(kind)), diagnostic: message)
        case .Filter(let kind, let message): GewuKitError(.filter(.init(kind)), diagnostic: message)
        case .Find(let kind, let message): GewuKitError(.find(.init(kind)), diagnostic: message)
        case .Analysis(let kind, let message): GewuKitError(.analysis(.init(kind)), diagnostic: message)
        case .Export(let kind, let message): GewuKitError(.export(.init(kind)), diagnostic: message)
        case .Internal(let message): GewuKitError(.internal, diagnostic: message)
        case .Cancelled: CancellationError()
        }
    }
}

extension GewuKitError.IoOperation {
    init(_ operation: GewuBindings.IoOperation) {
        self =
            switch operation {
            case .canonicalize: .canonicalize
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

extension GewuKitError.FileErrorKind {
    init(_ kind: GewuBindings.FileErrorKind) {
        self =
            switch kind {
            case .unknownFormat: .unknownFormat
            case .formatDisabled(let format): .formatDisabled(FileType(format))
            case .decode(let format): .decode(FileType(format))
            case .noWorksheets: .noWorksheets
            case .sheetNotFound(let sheet): .sheetNotFound(sheet)
            case .errorCell(let sheet, let row, let column, let error):
                .errorCell(sheet: sheet, row: row, column: column, error: error)
            case .unsupportedColumnType(let column, let dataType):
                .unsupportedColumnType(column: column, dataType: dataType)
            }
    }
}

extension GewuKitError.LimitErrorKind {
    init(_ kind: GewuBindings.LimitErrorKind) {
        self =
            switch kind {
            case .datasetRows(let actual, let limit): .datasetRows(actual: actual, limit: limit)
            case .xlsxRows(let actual, let limit): .xlsxRows(actual: actual, limit: limit)
            case .xlsxColumns(let actual, let limit): .xlsxColumns(actual: actual, limit: limit)
            }
    }
}

extension GewuKitError.FilterErrorKind {
    init(_ kind: GewuBindings.FilterErrorKind) {
        self =
            switch kind {
            case .invalidValue(let value, let expectedType, let reason):
                .invalidValue(value: value, expectedType: expectedType, reason: GewuKitError.ValueParseError(reason))
            case .reversedRange(let min, let max): .reversedRange(min: min, max: max)
            case .patternCompilationFailed: .patternCompilationFailed
            }
    }
}

extension GewuKitError.ValueParseError {
    init(_ reason: GewuBindings.ValueParseError) {
        self =
            switch reason {
            case .invalidSyntax: .invalidSyntax
            case .outOfRange: .outOfRange
            case .naNNotAllowed: .nanNotAllowed
            }
    }
}

extension GewuKitError.FindErrorKind {
    init(_ kind: GewuBindings.FindErrorKind) {
        self =
            switch kind {
            case .queryCompilationFailed: .queryCompilationFailed
            }
    }
}

extension GewuKitError.AnalysisErrorKind {
    init(_ kind: GewuBindings.AnalysisErrorKind) {
        self =
            switch kind {
            case .noFiniteValues: .noFiniteValues
            case .unrepresentableDomain(let dataType): .unrepresentableDomain(dataType: dataType)
            case .valueOutOfRange(let operation, let dataType):
                .valueOutOfRange(GewuKitError.AnalysisOperation(operation), dataType: dataType)
            case .unsupportedDataType(let operation, let dataType):
                .unsupportedDataType(GewuKitError.AnalysisOperation(operation), dataType: dataType)
            }
    }
}

extension GewuKitError.AnalysisOperation {
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

extension GewuKitError.ExportErrorKind {
    init(_ kind: GewuBindings.ExportErrorKind) {
        self =
            switch kind {
            case .sourceIsDestination(let sourcePath, let destination):
                .sourceIsDestination(sourcePath: sourcePath, destination: destination)
            case .nestedColumns(let format, let columns): .nestedColumns(FileType(format), columns.map(ColumnID.init))
            case .encode(let format): .encode(FileType(format))
            case .sizeLimit(let limit): .sizeLimit(limit)
            }
    }
}
