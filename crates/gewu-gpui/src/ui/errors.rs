//! User-facing wording is chosen at the UI boundary, where the engine error still has its type.
use gewu_core::{
    AnalysisError, BoundError, ColumnError, Error, ExportError, FileError, FilterError, FindError,
    IoError, IoOperation, ValueParseError,
};

use super::messages::format_name;

pub(crate) fn warning_icon() -> gpui_kit::component::Icon {
    use gpui_kit::{
        component::{Icon, Sizable, Size},
        px,
    };
    Icon::new(gpui_kit::assets::IconName::TriangleAlert).with_size(Size::Size(px(14.)))
}

pub(crate) fn inline_warning(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    use gpui_kit::{ParentElement, Styled, div};
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(warning_icon())
        .child(div().min_w_0().child(text.into()))
}

pub(crate) fn update_message(error: &crate::platform::update::HelperError) -> String {
    use crate::platform::update::HelperError;
    match error {
        HelperError::Launch(error) => format!("The update helper could not start: {error}"),
        HelperError::Failed { status, message } if message.is_empty() => {
            format!("Update helper exited with {status}")
        }
        HelperError::Failed { message, .. } => message.clone(),
        HelperError::Output(error) => {
            format!("The update helper reported unreadable output: {error}")
        }
        HelperError::Version(error) => format!("Invalid update version: {error}"),
        HelperError::Unexpected(version) => format!("Unexpected update version: {version}"),
    }
}

pub(crate) fn export_failure_message(failure: &crate::shell::ExportFailure) -> String {
    use crate::shell::ExportFailure;
    match failure {
        ExportFailure::Destination(error) => {
            format!("Could not choose export destination: {error}")
        }
        ExportFailure::Write(error) => message(error),
    }
}

pub(crate) fn picker_message(error: &crate::shell::PickerError) -> String {
    use crate::shell::PickerError;
    match error {
        PickerError::NotMainThread => "The file picker needs the main thread.".into(),
        PickerError::Unavailable => "macOS could not open the file picker.".into(),
        PickerError::Prompt(error) => format!("Could not show the file picker. {error}"),
    }
}

pub(crate) fn copy_message(failure: &crate::table::CopyFailure) -> String {
    use crate::table::CopyFailure;
    match failure {
        CopyFailure::Encode(error) => clipboard_message(error),
        CopyFailure::Pasteboard(error) => pasteboard_message(*error).into(),
    }
}

fn pasteboard_message(error: crate::platform::clipboard::PasteboardError) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        use crate::platform::clipboard::PasteboardError;
        match error {
            PasteboardError::Prepare => "Could not prepare the clipboard contents",
            PasteboardError::Write => "Could not write to the clipboard",
        }
    }
    #[cfg(not(target_os = "macos"))]
    match error {}
}

pub(crate) const SELECT_CELL_TO_FILTER: &str = "Select a cell to filter by its value";

pub(crate) fn open_reason(error: &Error) -> String {
    match error {
        Error::File(FileError::Decode { format, .. }) => {
            format!(
                "The file could not be read as {}. It may be damaged or use a different format.",
                format_name(*format)
            )
        }
        Error::Io(e) if e.source.kind() == std::io::ErrorKind::NotFound => {
            "The file could not be found. It may have been moved or renamed.".into()
        }
        Error::Io(e) if e.source.kind() == std::io::ErrorKind::PermissionDenied => {
            "Gewu does not have permission to read this file.".into()
        }
        _ => message(error),
    }
}

/// Clipboard export has a stricter size cap and a different recovery action from file export.
fn clipboard_message(error: &Error) -> String {
    match error {
        Error::Export(ExportError::SizeLimit { limit }) => format!(
            "The selection exceeds the {} MiB clipboard limit. Use File > Export to save it.",
            limit / (1024 * 1024)
        ),
        _ => message(error),
    }
}

/// Every engine error in words. Each kind is listed, so a new kind fails to compile until it
/// is worded. Contract violations, which no user action causes, share one internal message.
pub(crate) fn message(error: &Error) -> String {
    match error {
        Error::Io(e) => io_message(e),
        Error::File(e) => file_message(e),
        Error::Filter(e) => filter_message(e),
        Error::Find(e) => find_message(e),
        Error::Analysis(e) => analysis_message(e),
        Error::Export(e) => export_message(e),
        Error::Column(e) => column_message(e),
        Error::Cancelled => "Cancelled".into(),
        Error::Internal(e) => internal(e),
    }
}

/// A broken contract between Gewu's layers. The engine's text helps a bug report.
fn internal(error: &dyn std::fmt::Display) -> String {
    format!("Gewu encountered an internal error. {error}")
}

fn io_message(e: &IoError) -> String {
    let action = match e.operation {
        IoOperation::Canonicalize | IoOperation::ReadMetadata | IoOperation::Open => "open",
        IoOperation::Seek | IoOperation::Read => "read",
        IoOperation::CreateTemporaryFile | IoOperation::Sync | IoOperation::Commit => "save",
    };
    format!("Could not {action} {}. {}", e.path.display(), e.source)
}

fn file_message(error: &FileError) -> String {
    match error {
        FileError::UnknownFormat => "Choose a Parquet, CSV, TSV or Excel file.".into(),
        FileError::FormatDisabled { format } => {
            format!(
                "This build of Gewu cannot open {} files.",
                format_name(*format)
            )
        }
        FileError::Decode { format, source } => {
            format!(
                "Could not read the file as {}. {source}",
                format_name(*format)
            )
        }
        FileError::NoWorksheets => "The workbook has no worksheets.".into(),
        FileError::SheetNotFound { sheet } => {
            format!("The workbook has no worksheet named “{sheet}”.")
        }
        FileError::ErrorCell {
            sheet,
            row,
            column,
            error,
        } => format!("The worksheet “{sheet}” contains {error} at row {row}, column {column}."),
        FileError::UnsupportedColumnType { column, data_type } => {
            format!("Gewu cannot show the column “{column}” of type {data_type}.")
        }
        FileError::DatasetRows { actual, limit } => format!(
            "The table has {} rows. Gewu can open at most {}.",
            super::messages::format_count(*actual),
            super::messages::format_count(*limit)
        ),
        FileError::Io(e) => io_message(e),
        FileError::UnexpectedFormat { .. } => internal(error),
        FileError::Internal(e) => internal(e),
    }
}

fn invalid_value(
    value: &str,
    expected: &impl std::fmt::Display,
    reason: ValueParseError,
) -> String {
    match reason {
        ValueParseError::InvalidSyntax => format!("“{value}” is not a valid {expected} value."),
        ValueParseError::OutOfRange => format!("“{value}” is outside the range of {expected}."),
        ValueParseError::NaNNotAllowed => format!("“{value}” cannot be used as a bound."),
    }
}

fn bound_message(error: &BoundError) -> String {
    match error {
        BoundError::InvalidValue {
            value,
            expected,
            reason,
        } => invalid_value(value, expected, *reason),
        BoundError::UnsupportedType { data_type } => {
            format!("A range cannot filter {data_type} values.")
        }
        BoundError::OutOfRange { data_type } => {
            format!("The bound is outside the range of {data_type}.")
        }
        BoundError::Arrow(_) => internal(error),
    }
}

fn filter_message(error: &FilterError) -> String {
    match error {
        FilterError::InvalidValue {
            value,
            expected,
            reason,
        } => invalid_value(value, expected, *reason),
        FilterError::ReversedRange { min, max } => {
            format!("The minimum {min} is greater than the maximum {max}.")
        }
        FilterError::PatternCompilationFailed { .. } => {
            "The filter text is too complex to match.".into()
        }
        FilterError::UnsupportedType { data_type, .. } => {
            format!("This filter does not apply to {data_type} values.")
        }
        FilterError::Bound(e) => bound_message(e),
        FilterError::Column(e) => column_message(e),
        FilterError::RangeTypeMismatch { .. } => internal(error),
        FilterError::Internal(e) => internal(e),
    }
}

fn find_message(error: &FindError) -> String {
    match error {
        FindError::QueryCompilationFailed { .. } => {
            "The search text is too complex to match.".into()
        }
        FindError::EmptyQuery => "Enter text to find.".into(),
        FindError::Filter(e) => filter_message(e),
        FindError::InvalidPageSize { .. }
        | FindError::InvalidHighlightRows { .. }
        | FindError::InvalidCursorRow { .. }
        | FindError::InvalidCursorColumn { .. }
        | FindError::InvalidHighlightColumn { .. } => internal(error),
        FindError::Internal(e) => internal(e),
    }
}

fn analysis_message(error: &AnalysisError) -> String {
    match error {
        AnalysisError::NoFiniteValues => "There are no finite values to analyze.".into(),
        AnalysisError::UnrepresentableDomain { data_type } => {
            format!("These {data_type} values cannot be placed on an axis.")
        }
        AnalysisError::ValueOutOfRange { data_type, .. } => {
            format!("The result exceeds the range of {data_type}.")
        }
        AnalysisError::UnsupportedDataType { data_type, .. }
        | AnalysisError::UnsupportedOperation { data_type, .. } => {
            format!("This analysis does not support {data_type} values.")
        }
        AnalysisError::UnsupportedAxisScale { .. } => {
            "This axis does not support a logarithmic scale.".into()
        }
        AnalysisError::Bound(e) => bound_message(e),
        AnalysisError::Column(e) => column_message(e),
        AnalysisError::Filter(e) => filter_message(e),
        AnalysisError::InvalidBinCount { .. } | AnalysisError::InvalidScatterLimit { .. } => {
            internal(error)
        }
        AnalysisError::Internal(e) => internal(e),
    }
}

fn export_message(error: &ExportError) -> String {
    match error {
        ExportError::SourceIsDestination { .. } => {
            "The export would replace its source file. Choose another destination.".into()
        }
        ExportError::NestedColumns { format, .. } => format!(
            "Choose Parquet to preserve nested values, or export them as text in {}.",
            format_name(*format)
        ),
        ExportError::Encode { format, source } => {
            format!(
                "Could not write the data as {}. {source}",
                format_name(*format)
            )
        }
        ExportError::FormatDisabled { format } => {
            format!(
                "This build of Gewu cannot write {} files.",
                format_name(*format)
            )
        }
        ExportError::SizeLimit { limit } => format!(
            "The export exceeds the {} MiB limit.",
            limit / (1024 * 1024)
        ),
        ExportError::XlsxRows { actual, limit } => format!(
            "An Excel worksheet holds at most {} rows; this export has {}.",
            super::messages::format_count(*limit),
            super::messages::format_count(*actual)
        ),
        ExportError::XlsxColumns { actual, limit } => {
            format!("An Excel worksheet holds at most {limit} columns; this export has {actual}.")
        }
        ExportError::Io(e) => io_message(e),
        ExportError::Cancelled => "Cancelled".into(),
        ExportError::AlreadyStarted => internal(error),
        ExportError::Internal(e) => internal(e),
    }
}

fn column_message(error: &ColumnError) -> String {
    match error {
        ColumnError::EmptySelection { .. } => "Select at least one column.".into(),
        ColumnError::NotFound { .. } | ColumnError::Duplicate { .. } => internal(error),
        ColumnError::Internal(e) => internal(e),
    }
}

#[cfg(test)]
mod tests {
    use gewu_core::{AnalysisError, Error, IoError, IoOperation};

    #[test]
    fn opening_a_file_and_an_unavailable_analysis_get_distinct_messages() {
        let denied = Error::from(IoError {
            operation: IoOperation::Open,
            path: "measurements.csv".into(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
        });
        assert_eq!(
            super::message(&denied),
            "Could not open measurements.csv. permission denied"
        );
        assert_eq!(
            super::message(&Error::from(AnalysisError::NoFiniteValues)),
            "There are no finite values to analyze."
        );
    }

    #[test]
    fn a_search_beyond_the_matchers_compilation_limit_has_a_readable_error() {
        let source = regex::RegexBuilder::new("[a-z]{100}")
            .size_limit(1)
            .build()
            .unwrap_err();
        let error = gewu_core::FindError::QueryCompilationFailed { source };
        assert_eq!(
            super::message(&error.into()),
            "The search text is too complex to match."
        );
    }

    #[test]
    fn invalid_filter_during_analysis_retains_the_filter_message() {
        let error = gewu_core::AnalysisError::Filter(gewu_core::FilterError::ReversedRange {
            min: "11".into(),
            max: "4".into(),
        });
        assert_eq!(
            super::message(&error.into()),
            "The minimum 11 is greater than the maximum 4."
        );
    }
}
