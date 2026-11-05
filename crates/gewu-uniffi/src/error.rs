//! Structured errors crossing the language boundary.

use gewu_core as core;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum GewuError {
    #[error("{message}")]
    Io {
        operation: IoOperation,
        message: String,
    },
    #[error("{message}")]
    File {
        kind: FileErrorKind,
        message: String,
    },
    #[error("{message}")]
    Filter {
        kind: FilterErrorKind,
        message: String,
    },
    #[error("{message}")]
    Find {
        kind: FindErrorKind,
        message: String,
    },
    #[error("{message}")]
    Analysis {
        kind: AnalysisErrorKind,
        message: String,
    },
    #[error("{message}")]
    Export {
        kind: ExportErrorKind,
        message: String,
    },
    #[error("{message}")]
    Limit {
        kind: LimitErrorKind,
        message: String,
    },
    #[error("cancelled")]
    Cancelled,
    #[error("{message}")]
    Internal { message: String },
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoOperation {
    Canonicalize,
    ReadMetadata,
    Open,
    Read,
    Seek,
    CreateTemporaryFile,
    Sync,
    Commit,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum FileErrorKind {
    UnknownFormat,
    FormatDisabled {
        format: crate::FileType,
    },
    Decode {
        format: crate::FileType,
    },
    NoWorksheets,
    SheetNotFound {
        sheet: String,
    },
    ErrorCell {
        sheet: String,
        row: u32,
        column: u32,
    },
    UnsupportedColumnType {
        column: String,
        data_type: String,
    },
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum FilterErrorKind {
    InvalidValue {
        value: String,
        expected_type: String,
        reason: ValueParseError,
    },
    ReversedRange {
        min: String,
        max: String,
    },
    PatternCompilationFailed,
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueParseError {
    InvalidSyntax,
    OutOfRange,
    NaNNotAllowed,
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindErrorKind {
    QueryCompilationFailed,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum AnalysisErrorKind {
    NoFiniteValues,
    UnrepresentableDomain {
        data_type: String,
    },
    ValueOutOfRange {
        operation: AnalysisOperation,
        data_type: String,
    },
    UnsupportedDataType {
        operation: AnalysisOperation,
        data_type: String,
    },
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisOperation {
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
    Scatter,
    FilterDistribution,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum ExportErrorKind {
    SourceIsDestination {
        source_path: String,
        destination: String,
    },
    NestedColumns {
        format: crate::FileType,
        columns: Vec<u32>,
    },
    Encode {
        format: crate::FileType,
    },
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitErrorKind {
    DatasetRows { actual: u64, limit: u64 },
    XlsxRows { actual: u64, limit: u64 },
    XlsxColumns { actual: u32, limit: u32 },
}

impl From<core::Error> for GewuError {
    fn from(error: core::Error) -> Self {
        let message = error.to_string();
        match error {
            core::Error::Io(error) => Self::Io {
                operation: error.operation.into(),
                message,
            },
            core::Error::File(kind) => file_error(kind, message),
            core::Error::Filter(kind) => filter_error(kind, message),
            core::Error::Find(kind) => match kind {
                core::FindError::QueryCompilationFailed { .. } => Self::Find {
                    kind: FindErrorKind::QueryCompilationFailed,
                    message,
                },
                core::FindError::Filter(kind) => filter_error(kind, message),
                core::FindError::Cancelled(_) => Self::Cancelled,
                core::FindError::EmptyQuery
                | core::FindError::InvalidPageSize { .. }
                | core::FindError::InvalidHighlightRows { .. }
                | core::FindError::InvalidCursorRow { .. }
                | core::FindError::InvalidCursorColumn { .. }
                | core::FindError::Internal(_) => Self::Internal { message },
            },
            core::Error::Analysis(kind) => analysis_error(kind, message),
            core::Error::Export(kind) => export_error(kind, message),
            core::Error::Column(_) | core::Error::Query(_) | core::Error::Internal(_) => {
                Self::Internal { message }
            }
            core::Error::Cancelled => Self::Cancelled,
        }
    }
}

impl From<core::FindError> for GewuError {
    fn from(error: core::FindError) -> Self {
        core::Error::from(error).into()
    }
}

fn file_error(kind: core::FileError, message: String) -> GewuError {
    let kind = match kind {
        core::FileError::UnknownFormat => FileErrorKind::UnknownFormat,
        core::FileError::FormatDisabled { format } => FileErrorKind::FormatDisabled {
            format: format.into(),
        },
        core::FileError::Decode { format, .. } => FileErrorKind::Decode {
            format: format.into(),
        },
        core::FileError::NoWorksheets => FileErrorKind::NoWorksheets,
        core::FileError::SheetNotFound { sheet } => FileErrorKind::SheetNotFound { sheet },
        core::FileError::ErrorCell {
            sheet, row, column, ..
        } => FileErrorKind::ErrorCell { sheet, row, column },
        core::FileError::UnsupportedColumnType { column, data_type } => {
            FileErrorKind::UnsupportedColumnType {
                column,
                data_type: data_type.to_string(),
            }
        }
        core::FileError::DatasetRows { actual, limit } => {
            return GewuError::Limit {
                kind: LimitErrorKind::DatasetRows { actual, limit },
                message,
            };
        }
        core::FileError::UnexpectedFormat { .. } | core::FileError::Internal(_) => {
            return GewuError::Internal { message };
        }
        core::FileError::Io(error) => {
            return GewuError::Io {
                operation: error.operation.into(),
                message,
            };
        }
    };
    GewuError::File { kind, message }
}

fn filter_error(kind: core::FilterError, message: String) -> GewuError {
    let kind = match kind {
        core::FilterError::InvalidValue {
            value,
            expected,
            reason,
        } => FilterErrorKind::InvalidValue {
            value,
            expected_type: expected.to_string(),
            reason: reason.into(),
        },
        core::FilterError::Bound(core::BoundError::InvalidValue {
            value,
            expected,
            reason,
        }) => FilterErrorKind::InvalidValue {
            value,
            expected_type: expected.to_string(),
            reason: reason.into(),
        },
        core::FilterError::ReversedRange { min, max } => {
            FilterErrorKind::ReversedRange { min, max }
        }
        core::FilterError::PatternCompilationFailed { .. } => {
            FilterErrorKind::PatternCompilationFailed
        }
        core::FilterError::Cancelled(_) => return GewuError::Cancelled,
        core::FilterError::Bound(_)
        | core::FilterError::UnsupportedType { .. }
        | core::FilterError::RangeTypeMismatch { .. }
        | core::FilterError::Column(_)
        | core::FilterError::Internal(_) => return GewuError::Internal { message },
    };
    GewuError::Filter { kind, message }
}

fn analysis_error(kind: core::AnalysisError, message: String) -> GewuError {
    let kind = match kind {
        core::AnalysisError::NoFiniteValues => AnalysisErrorKind::NoFiniteValues,
        core::AnalysisError::UnrepresentableDomain { data_type } => {
            AnalysisErrorKind::UnrepresentableDomain {
                data_type: data_type.to_string(),
            }
        }
        core::AnalysisError::ValueOutOfRange {
            operation,
            data_type,
        } => AnalysisErrorKind::ValueOutOfRange {
            operation: operation.into(),
            data_type: data_type.to_string(),
        },
        core::AnalysisError::UnsupportedDataType {
            operation,
            data_type,
        } => AnalysisErrorKind::UnsupportedDataType {
            operation: operation.into(),
            data_type: data_type.to_string(),
        },
        core::AnalysisError::Cancelled(_) => return GewuError::Cancelled,
        core::AnalysisError::InvalidBinCount { .. }
        | core::AnalysisError::InvalidScatterLimit { .. }
        | core::AnalysisError::UnsupportedOperation { .. }
        | core::AnalysisError::UnsupportedAxisScale { .. }
        | core::AnalysisError::Bound(_)
        | core::AnalysisError::Filter(_)
        | core::AnalysisError::Column(_)
        | core::AnalysisError::Internal(_) => return GewuError::Internal { message },
    };
    GewuError::Analysis { kind, message }
}

fn export_error(kind: core::ExportError, message: String) -> GewuError {
    match kind {
        core::ExportError::SourceIsDestination {
            source_path,
            destination,
        } => GewuError::Export {
            kind: ExportErrorKind::SourceIsDestination {
                source_path: source_path.display().to_string(),
                destination: destination.display().to_string(),
            },
            message,
        },
        core::ExportError::NestedColumns { format, columns } => GewuError::Export {
            kind: ExportErrorKind::NestedColumns {
                format: format.into(),
                columns,
            },
            message,
        },
        core::ExportError::Encode { format, .. } => GewuError::Export {
            kind: ExportErrorKind::Encode {
                format: format.into(),
            },
            message,
        },
        core::ExportError::FormatDisabled { format } => GewuError::File {
            kind: FileErrorKind::FormatDisabled {
                format: format.into(),
            },
            message,
        },
        core::ExportError::XlsxRows { actual, limit } => GewuError::Limit {
            kind: LimitErrorKind::XlsxRows { actual, limit },
            message,
        },
        core::ExportError::XlsxColumns { actual, limit } => GewuError::Limit {
            kind: LimitErrorKind::XlsxColumns { actual, limit },
            message,
        },
        core::ExportError::Io(error) => GewuError::Io {
            operation: error.operation.into(),
            message,
        },
        core::ExportError::Cancelled(_) => GewuError::Cancelled,
        core::ExportError::AlreadyStarted | core::ExportError::Internal(_) => {
            GewuError::Internal { message }
        }
    }
}

impl From<core::IoOperation> for IoOperation {
    fn from(value: core::IoOperation) -> Self {
        match value {
            core::IoOperation::Canonicalize => Self::Canonicalize,
            core::IoOperation::ReadMetadata => Self::ReadMetadata,
            core::IoOperation::Open => Self::Open,
            core::IoOperation::Read => Self::Read,
            core::IoOperation::Seek => Self::Seek,
            core::IoOperation::CreateTemporaryFile => Self::CreateTemporaryFile,
            core::IoOperation::Sync => Self::Sync,
            core::IoOperation::Commit => Self::Commit,
        }
    }
}

impl From<core::ValueParseError> for ValueParseError {
    fn from(value: core::ValueParseError) -> Self {
        match value {
            core::ValueParseError::InvalidSyntax => Self::InvalidSyntax,
            core::ValueParseError::OutOfRange => Self::OutOfRange,
            core::ValueParseError::NaNNotAllowed => Self::NaNNotAllowed,
        }
    }
}

impl From<core::AnalysisOperation> for AnalysisOperation {
    fn from(value: core::AnalysisOperation) -> Self {
        match value {
            core::AnalysisOperation::Statistics => Self::Statistics,
            core::AnalysisOperation::Distribution => Self::Distribution,
            core::AnalysisOperation::Values => Self::Values,
            core::AnalysisOperation::Lengths => Self::Lengths,
            core::AnalysisOperation::Structure => Self::Structure,
            core::AnalysisOperation::Scatter => Self::Scatter,
            core::AnalysisOperation::FilterDistribution => Self::FilterDistribution,
        }
    }
}
