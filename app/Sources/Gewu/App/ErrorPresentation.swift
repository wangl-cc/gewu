import Foundation
import GewuKit

/// How the App words a failure for the user: a message, plus details when the
/// message alone cannot say what happened. Every place that shows a failure
/// shows ``text``, so a failure reads the same in an alert and inline.
///
/// GewuKit reports engine failures as structured kinds without wording; this
/// is the one place that turns them into English.
struct ErrorPresentation {
    let message: String
    let details: String?

    init(_ error: any Error) {
        if let error = error as? GewuKitError {
            message = error.kind.message
            details = error.kind.needsDiagnostic ? error.diagnostic : nil
        } else {
            message = error.localizedDescription
            details = (error as? LocalizedError)?.failureReason
        }
    }

    /// The message, followed by the details when there are any.
    var text: String {
        guard let details else { return message }
        return message + "\n\n" + details
    }
}

extension GewuKitError.Kind {
    /// Whether only the engine's diagnostic says what happened, such as a
    /// parser's or the file system's message.
    fileprivate var needsDiagnostic: Bool {
        switch self {
        case .io, .file(.decode), .export(.encode), .internal: true
        default: false
        }
    }

    fileprivate var message: String {
        switch self {
        case .io(let operation):
            switch operation {
            case .canonicalize, .readMetadata, .open: "The file could not be opened."
            case .read, .seek: "The file could not be read."
            case .createTemporaryFile, .sync, .commit: "The file could not be saved."
            }
        case .file(let kind):
            switch kind {
            case .unknownFormat: "The file is not a Parquet, CSV, TSV or XLSX file."
            case .formatDisabled(let format): "This build cannot read \(format.displayName) files."
            case .decode(let format): "The file could not be read as \(format.displayName)."
            case .noWorksheets: "The workbook has no worksheets."
            case .sheetNotFound(let sheet): "The workbook has no worksheet named “\(sheet)”."
            case .errorCell(let sheet, let row, let column, let error):
                "Worksheet “\(sheet)” has the error value \(error) at row \(row), column \(column)."
            case .unsupportedColumnType(let column, let dataType):
                "Column “\(column)” has a type Gewu cannot show (\(dataType))."
            }
        case .limit(let kind):
            switch kind {
            case .datasetRows(let actual, let limit):
                "The table has \(actual) rows; Gewu supports at most \(limit)."
            case .xlsxRows(let actual, let limit):
                "A worksheet holds at most \(limit) rows, not \(actual)."
            case .xlsxColumns(let actual, let limit):
                "A worksheet holds at most \(limit) columns, not \(actual)."
            }
        case .filter(let kind):
            switch kind {
            case .invalidValue(let value, let type, .invalidSyntax): "“\(value)” is not a valid \(type) value."
            case .invalidValue(let value, let type, .outOfRange): "“\(value)” is out of range for \(type)."
            case .invalidValue(let value, _, .nanNotAllowed): "“\(value)” cannot be used as a bound."
            case .reversedRange(let min, let max): "The minimum \(min) is greater than the maximum \(max)."
            case .patternCompilationFailed: "The filter text is too complex to match."
            }
        case .find(.queryCompilationFailed): "The search text is too complex to match."
        case .analysis(let kind):
            switch kind {
            case .noFiniteValues: "There are no finite values to analyze."
            case .unrepresentableDomain(let type): "These \(type) values cannot be placed on an axis."
            case .valueOutOfRange(let operation, let type):
                "\(operation.displayName): some \(type) values are out of range."
            case .unsupportedDataType(let operation, let type):
                "\(operation.displayName) is not available for \(type) columns."
            }
        case .export(let kind):
            switch kind {
            case .sourceIsDestination: "The export would replace the file it reads from."
            case .nestedColumns(let format, _): "\(format.displayName) cannot hold nested columns."
            case .encode(let format): "The data could not be written as \(format.displayName)."
            case .sizeLimit(let limit):
                "The result is larger than \(Int64(clamping: limit).formatted(.byteCount(style: .memory)))."
            }
        case .internal: "Gewu encountered an internal error."
        }
    }
}

extension GewuKitError.AnalysisOperation {
    fileprivate var displayName: String {
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
