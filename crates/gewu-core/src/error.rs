//! The engine's error type, divided by the operation that failed. Nested internal errors
//! surface at the top level, and only broken contracts are internal.
use std::path::PathBuf;

use thiserror::Error;

use crate::{AnalysisError, ColumnError, ExportError, FileError, FilterError, FindError};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    File(FileError),
    #[error(transparent)]
    Column(ColumnError),
    #[error(transparent)]
    Filter(FilterError),
    #[error(transparent)]
    Find(FindError),
    #[error(transparent)]
    Analysis(AnalysisError),
    #[error(transparent)]
    Export(ExportError),
    #[error(transparent)]
    Internal(#[from] InternalError),
    /// The export was cancelled before destination commit.
    #[error("cancelled")]
    Cancelled,
}

impl From<FileError> for Error {
    fn from(error: FileError) -> Self {
        match error {
            FileError::Io(error) => Self::Io(error),
            FileError::Internal(error) => Self::Internal(error),
            other => Self::File(other),
        }
    }
}

impl From<ColumnError> for Error {
    fn from(error: ColumnError) -> Self {
        match error {
            ColumnError::Internal(error) => Self::Internal(error),
            other => Self::Column(other),
        }
    }
}

impl From<FilterError> for Error {
    fn from(error: FilterError) -> Self {
        match error {
            FilterError::Column(error) => error.into(),
            FilterError::Internal(error) => Self::Internal(error),
            other => Self::Filter(other),
        }
    }
}

impl From<FindError> for Error {
    fn from(error: FindError) -> Self {
        match error {
            FindError::Filter(error) => error.into(),
            FindError::Internal(error) => Self::Internal(error),
            other => Self::Find(other),
        }
    }
}

impl From<AnalysisError> for Error {
    fn from(error: AnalysisError) -> Self {
        match error {
            AnalysisError::Filter(error) => error.into(),
            AnalysisError::Column(error) => error.into(),
            AnalysisError::Internal(error) => Self::Internal(error),
            other => Self::Analysis(other),
        }
    }
}

impl From<ExportError> for Error {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Io(error) => Self::Io(error),
            ExportError::Cancelled => Self::Cancelled,
            ExportError::Internal(error) => Self::Internal(error),
            other => Self::Export(other),
        }
    }
}

#[derive(Debug, Error)]
#[error("I/O error during {operation:?} on {path:?}: {source}")]
pub struct IoError {
    pub operation: IoOperation,
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

impl IoError {
    pub(crate) fn new(
        operation: IoOperation,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self {
            operation,
            path: path.into(),
            source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Error)]
pub enum InternalError {
    #[error("Arrow error: {0}")]
    Arrow(arrow::error::ArrowError),
    #[error(transparent)]
    Codec(crate::CodecError),
    #[error("internal invariant violated: {0}")]
    Invariant(String),
}

impl From<arrow::error::ArrowError> for InternalError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Arrow(source)
    }
}

impl From<arrow::error::ArrowError> for Error {
    fn from(source: arrow::error::ArrowError) -> Self {
        InternalError::Arrow(source).into()
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
