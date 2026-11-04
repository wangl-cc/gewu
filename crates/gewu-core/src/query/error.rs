use thiserror::Error;

use crate::{CancellationError, ColumnError, FilterError, InternalError};

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("duplicate sort priority {priority}")]
    DuplicateSortPriority { priority: u32 },
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl From<arrow::error::ArrowError> for QueryError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}

impl From<crate::execution::sort::SortError> for QueryError {
    fn from(error: crate::execution::sort::SortError) -> Self {
        match error {
            crate::execution::sort::SortError::Cancelled(error) => Self::Cancelled(error),
            crate::execution::sort::SortError::Arrow(error) => Self::from(error),
        }
    }
}
