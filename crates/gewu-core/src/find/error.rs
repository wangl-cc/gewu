use thiserror::Error;

use crate::{CancellationError, FilterError, InternalError};

#[derive(Debug, Error)]
pub enum FindError {
    #[error("cannot compile text search: {source}")]
    QueryCompilationFailed {
        #[source]
        source: regex::Error,
    },
    #[error("find query cannot be empty")]
    EmptyQuery,
    #[error("find page size must be between {min} and {max}; received {value}")]
    InvalidPageSize { value: u32, min: u32, max: u32 },
    #[error("find highlight rows must be between {min} and {max}; received {value}")]
    InvalidHighlightRows { value: u32, min: u32, max: u32 },
    #[error("find cursor row {row} is outside the captured view with {row_count} rows")]
    InvalidCursorRow { row: u64, row_count: u64 },
    #[error("find cursor column {column} is outside the captured scope")]
    InvalidCursorColumn { column: u32 },
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl From<arrow::error::ArrowError> for FindError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}
