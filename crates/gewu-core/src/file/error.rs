use arrow::datatypes::DataType;
use thiserror::Error;

use crate::{CancellationError, InternalError, IoError};

#[derive(Debug, Error)]
pub enum FileError {
    #[error("expected a Parquet, CSV, TSV or XLSX file; specify a format for other extensions")]
    UnknownFormat,
    #[error("{format} support is disabled in this build")]
    FormatDisabled { format: crate::FileType },
    #[error("could not decode {format}: {source}")]
    Decode {
        format: crate::FileType,
        #[source]
        source: CodecError,
    },
    #[error("XLSX workbook contains no worksheets")]
    NoWorksheets,
    #[error("XLSX worksheet {sheet:?} does not exist")]
    SheetNotFound { sheet: String },
    #[error("XLSX worksheet {sheet:?}, row {row}, column {column} contains {error}")]
    ErrorCell {
        sheet: String,
        row: u32,
        column: u32,
        error: String,
    },
    #[error("column {column:?} has unsupported type {data_type}")]
    UnsupportedColumnType { column: String, data_type: DataType },
    #[error("dataset has {actual} rows, exceeding the supported limit of {limit}")]
    DatasetRows { actual: u64, limit: u64 },
    #[error("{operation:?} requires {expected}, not {actual}")]
    UnexpectedFormat {
        operation: FileOperation,
        expected: crate::FileType,
        actual: crate::FileType,
    },
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl FileError {
    pub(crate) fn decode(format: crate::FileType, error: Self) -> Self {
        match error {
            Self::Internal(InternalError::Arrow(source)) => Self::Decode {
                format,
                source: CodecError::Arrow(source),
            },
            Self::Internal(InternalError::Codec(source)) => Self::Decode { format, source },
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperation {
    SheetListing,
    SheetSelection,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("source file {source_path:?} and destination {destination:?} must be different")]
    SourceIsDestination {
        source_path: std::path::PathBuf,
        destination: std::path::PathBuf,
    },
    #[error("{format} cannot export nested columns {columns:?}; use Parquet to preserve them")]
    NestedColumns {
        format: crate::FileType,
        columns: Vec<u32>,
    },
    #[error("could not encode {format}: {source}")]
    Encode {
        format: crate::FileType,
        #[source]
        source: CodecError,
    },
    #[error("{format} support is disabled in this build")]
    FormatDisabled { format: crate::FileType },
    #[error("this export has already been started")]
    AlreadyStarted,
    #[error("XLSX export has {actual} data rows, exceeding the limit of {limit}")]
    XlsxRows { actual: u64, limit: u64 },
    #[error("XLSX export has {actual} columns, exceeding the limit of {limit}")]
    XlsxColumns { actual: u32, limit: u32 },
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl ExportError {
    pub(crate) fn encode(format: crate::FileType, error: Self) -> Self {
        match error {
            Self::Internal(InternalError::Arrow(source)) => Self::Encode {
                format,
                source: CodecError::Arrow(source),
            },
            Self::Internal(InternalError::Codec(source)) => Self::Encode { format, source },
            other => other,
        }
    }
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("Arrow error: {0}")]
    Arrow(arrow::error::ArrowError),
    #[cfg(feature = "csv")]
    #[error("CSV/TSV error: {0}")]
    Csv(csv::Error),
    #[cfg(feature = "parquet")]
    #[error("Parquet error: {0}")]
    Parquet(parquet::errors::ParquetError),
    #[cfg(feature = "xlsx")]
    #[error("XLSX read error: {0}")]
    XlsxRead(calamine::XlsxError),
    #[cfg(feature = "xlsx")]
    #[error("XLSX write error: {0}")]
    XlsxWrite(rust_xlsxwriter::XlsxError),
}

impl From<arrow::error::ArrowError> for FileError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}

impl From<arrow::error::ArrowError> for ExportError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}

#[cfg(feature = "csv")]
impl From<csv::Error> for FileError {
    fn from(source: csv::Error) -> Self {
        Self::Internal(InternalError::Codec(CodecError::Csv(source)))
    }
}

#[cfg(feature = "csv")]
impl From<csv::Error> for ExportError {
    fn from(source: csv::Error) -> Self {
        Self::Internal(InternalError::Codec(CodecError::Csv(source)))
    }
}

#[cfg(feature = "parquet")]
impl From<parquet::errors::ParquetError> for FileError {
    fn from(source: parquet::errors::ParquetError) -> Self {
        Self::Internal(InternalError::Codec(CodecError::Parquet(source)))
    }
}

#[cfg(feature = "parquet")]
impl From<parquet::errors::ParquetError> for ExportError {
    fn from(source: parquet::errors::ParquetError) -> Self {
        Self::Internal(InternalError::Codec(CodecError::Parquet(source)))
    }
}

#[cfg(feature = "xlsx")]
impl From<calamine::XlsxError> for FileError {
    fn from(source: calamine::XlsxError) -> Self {
        Self::Internal(InternalError::Codec(CodecError::XlsxRead(source)))
    }
}

#[cfg(feature = "xlsx")]
impl From<rust_xlsxwriter::XlsxError> for ExportError {
    fn from(source: rust_xlsxwriter::XlsxError) -> Self {
        Self::Internal(InternalError::Codec(CodecError::XlsxWrite(source)))
    }
}
