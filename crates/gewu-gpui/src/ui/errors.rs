//! User-facing wording is chosen at the UI boundary, where the engine error still has its type.
use gewu_core::{
    AnalysisError, Error, ExportError, FileError, FilterError, FindError, IoOperation,
    ValueParseError,
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

pub(crate) fn pasteboard_message(
    error: crate::platform::clipboard::PasteboardError,
) -> &'static str {
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
pub(crate) fn clipboard_message(error: &Error) -> String {
    match error {
        Error::Export(ExportError::SizeLimit { limit }) => format!(
            "The selection exceeds the {} MiB clipboard limit. Use File > Export to save it.",
            limit / (1024 * 1024)
        ),
        _ => message(error),
    }
}

pub(crate) fn message(error: &Error) -> String {
    match error {
        Error::Io(e) => {
            let action = match e.operation {
                IoOperation::Canonicalize | IoOperation::ReadMetadata | IoOperation::Open => "open",
                IoOperation::Seek | IoOperation::Read => "read",
                IoOperation::CreateTemporaryFile | IoOperation::Sync | IoOperation::Commit => {
                    "save"
                }
            };
            format!("Could not {action} {}. {}", e.path.display(), e.source)
        }
        Error::File(FileError::UnknownFormat) => "Choose a Parquet, CSV, TSV or Excel file.".into(),
        Error::File(FileError::NoWorksheets) => "The workbook has no worksheets.".into(),
        Error::File(FileError::Decode { format, source }) => {
            format!(
                "Could not read the file as {}. {source}",
                format_name(*format)
            )
        }
        Error::File(FileError::SheetNotFound { sheet }) => {
            format!("The workbook has no worksheet named “{sheet}”.")
        }
        Error::File(e) => e.to_string(),
        Error::Filter(FilterError::InvalidValue {
            value,
            expected,
            reason,
        }) => match reason {
            ValueParseError::InvalidSyntax => format!("“{value}” is not a valid {expected} value."),
            ValueParseError::OutOfRange => format!("“{value}” is outside the range of {expected}."),
            ValueParseError::NaNNotAllowed => format!("“{value}” cannot be used as a bound."),
        },
        Error::Filter(FilterError::ReversedRange { min, max }) => {
            format!("The minimum {min} is greater than the maximum {max}.")
        }
        Error::Filter(FilterError::PatternCompilationFailed { .. }) => {
            "The filter text is too complex to match.".into()
        }
        Error::Filter(e) => format!("Could not apply the filter. {e}"),
        Error::Find(FindError::QueryCompilationFailed { .. }) => {
            "The search text is too complex to match.".into()
        }
        Error::Find(e) => format!("Could not complete the search. {e}"),
        Error::Analysis(AnalysisError::NoFiniteValues) => {
            "There are no finite values to analyze.".into()
        }
        Error::Analysis(AnalysisError::UnrepresentableDomain { data_type }) => {
            format!("These {data_type} values cannot be placed on an axis.")
        }
        Error::Analysis(e) => e.to_string(),
        Error::Export(ExportError::SourceIsDestination { .. }) => {
            "The export would replace its source file. Choose another destination.".into()
        }
        Error::Export(ExportError::NestedColumns { format, .. }) => {
            format!(
                "Choose Parquet to preserve nested values, or export them as text in {}.",
                format_name(*format)
            )
        }
        Error::Export(ExportError::Encode { format, source }) => {
            format!(
                "Could not write the data as {}. {source}",
                format_name(*format)
            )
        }
        Error::Export(e) => e.to_string(),
        Error::Column(e) => format!("Could not use the selected columns. {e}"),
        Error::Cancelled => "Cancelled".into(),
        Error::Internal(e) => format!("Gewu encountered an internal error. {e}"),
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
