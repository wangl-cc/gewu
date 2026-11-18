use thiserror::Error;

use crate::{ColumnError, FilterError, InternalError};

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("duplicate sort priority {priority}")]
    DuplicateSortPriority { priority: u32 },
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

impl From<arrow::error::ArrowError> for QueryError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}
