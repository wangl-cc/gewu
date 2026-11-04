use std::path::PathBuf;

use thiserror::Error;

use crate::{
    AnalysisError, ColumnError, ExportError, FileError, FilterError, FindError, QueryError,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    File(FileError),
    #[error(transparent)]
    Column(ColumnError),
    #[error(transparent)]
    Query(QueryError),
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
    /// The operation was explicitly cancelled or replaced before completion.
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
            ColumnError::Cancelled(_) => Self::Cancelled,
            ColumnError::Internal(error) => Self::Internal(error),
            other => Self::Column(other),
        }
    }
}

impl From<QueryError> for Error {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::Column(error) => error.into(),
            QueryError::Filter(error) => error.into(),
            QueryError::Cancelled(_) => Self::Cancelled,
            QueryError::Internal(error) => Self::Internal(error),
            other => Self::Query(other),
        }
    }
}

impl From<FilterError> for Error {
    fn from(error: FilterError) -> Self {
        match error {
            FilterError::Column(error) => error.into(),
            FilterError::Cancelled(_) => Self::Cancelled,
            FilterError::Internal(error) => Self::Internal(error),
            other => Self::Filter(other),
        }
    }
}

impl From<FindError> for Error {
    fn from(error: FindError) -> Self {
        match error {
            FindError::Filter(error) => error.into(),
            FindError::Cancelled(_) => Self::Cancelled,
            FindError::Internal(error) => Self::Internal(error),
            other => Self::Find(other),
        }
    }
}

impl From<AnalysisError> for Error {
    fn from(error: AnalysisError) -> Self {
        match error {
            AnalysisError::Column(error) => error.into(),
            AnalysisError::Cancelled(_) => Self::Cancelled,
            AnalysisError::Internal(error) => Self::Internal(error),
            other => Self::Analysis(other),
        }
    }
}

impl From<ExportError> for Error {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Io(error) => Self::Io(error),
            ExportError::Cancelled(_) => Self::Cancelled,
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

impl From<crate::CancellationError> for Error {
    fn from(_: crate::CancellationError) -> Self {
        Self::Cancelled
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
