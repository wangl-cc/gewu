use arrow::datatypes::DataType;
use thiserror::Error;

use crate::{CancellationError, InternalError};

pub(crate) type Result<T> = std::result::Result<T, AnalysisError>;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("histogram needs at least one finite value")]
    NoFiniteValues,
    #[error("column of type {data_type} has no representable histogram domain")]
    UnrepresentableDomain { data_type: DataType },
    #[error("{operation:?} result exceeds the representable range of {data_type}")]
    ValueOutOfRange {
        operation: AnalysisOperation,
        data_type: DataType,
    },
    #[error("{operation:?} does not support {data_type}")]
    UnsupportedDataType {
        operation: AnalysisOperation,
        data_type: DataType,
    },
    #[error("{operation:?} bins must be between {min} and {max}; received {value}")]
    InvalidBinCount {
        operation: AnalysisOperation,
        value: u32,
        min: u32,
        max: u32,
    },
    #[error("scatter limit must be between {min} and {max}; received {value}")]
    InvalidScatterLimit { value: u32, min: u32, max: u32 },
    #[error("{operation:?} does not support {data_type}")]
    UnsupportedOperation {
        operation: AnalysisOperation,
        data_type: DataType,
    },
    #[error("{operation:?} does not support {scale:?} on this axis")]
    UnsupportedAxisScale {
        operation: AnalysisOperation,
        scale: crate::AxisScale,
    },
    #[error(transparent)]
    Bound(#[from] crate::BoundError),
    #[error(transparent)]
    Column(#[from] crate::ColumnError),
    #[error(transparent)]
    Filter(#[from] crate::FilterError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    Internal(#[from] InternalError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisOperation {
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
    Scatter,
    FilterDistribution,
}

impl From<arrow::error::ArrowError> for AnalysisError {
    fn from(source: arrow::error::ArrowError) -> Self {
        Self::Internal(InternalError::Arrow(source))
    }
}
