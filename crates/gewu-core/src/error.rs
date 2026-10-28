use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The operation was replaced by a newer one before it finished.
    #[error("cancelled")]
    Cancelled,

    /// The caller asked for rows of a view that is no longer current.
    #[error("stale view version {requested}, current is {current}")]
    StaleVersion { requested: u64, current: u64 },

    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
