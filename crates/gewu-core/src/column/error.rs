use arrow::datatypes::DataType;
use thiserror::Error;

use crate::{CancellationError, InternalError};

#[derive(Debug, Error)]
pub enum ColumnError {
    #[error("column {column} does not exist in a dataset with {column_count} columns")]
    NotFound { column: u32, column_count: u32 },
    #[error("duplicate column {column} in {operation:?}")]
    Duplicate {
        operation: ColumnOperation,
        column: u32,
    },
    #[error("column order {actual:?} must be a permutation of {expected:?}")]
    InvalidOrder {
        expected: Vec<u32>,
        actual: Vec<u32>,
    },
    #[error("{operation:?} requires at least one selected column")]
    EmptySelection { operation: ColumnOperation },
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnOperation {
    Query,
    Find,
    Export,
}

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("cannot read {value:?} as {expected}: {reason:?}")]
    InvalidValue {
        value: String,
        expected: DataType,
        reason: ValueParseError,
    },
    #[error("minimum {min:?} must not exceed maximum {max:?}")]
    ReversedRange { min: String, max: String },
    #[error("cannot compile text filter: {source}")]
    PatternCompilationFailed {
        #[source]
        source: regex::Error,
    },
    #[error("{condition:?} filter does not support {data_type}")]
    UnsupportedType {
        condition: FilterKind,
        data_type: DataType,
    },
    #[error("range endpoint of type {actual} cannot filter {expected}")]
    RangeTypeMismatch {
        expected: DataType,
        actual: DataType,
    },
    #[error(transparent)]
    Bound(#[from] crate::BoundError),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueParseError {
    InvalidSyntax,
    OutOfRange,
    NaNNotAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Exact,
    Range,
    Text,
}

impl From<&crate::FilterCondition> for FilterKind {
    fn from(condition: &crate::FilterCondition) -> Self {
        match condition {
            crate::FilterCondition::Exact { .. } => Self::Exact,
            crate::FilterCondition::Range { .. } => Self::Range,
            crate::FilterCondition::Text { .. } => Self::Text,
        }
    }
}

impl From<arrow::error::ArrowError> for ColumnError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}

impl From<arrow::error::ArrowError> for FilterError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}
