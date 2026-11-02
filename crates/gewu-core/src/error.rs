use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The operation was explicitly cancelled or replaced before completion.
    #[error("cancelled")]
    Cancelled,

    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[cfg(feature = "csv")]
    #[error("CSV/TSV error: {0}")]
    Csv(#[from] csv::Error),

    #[cfg(feature = "parquet")]
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[cfg(feature = "xlsx")]
    #[error("XLSX read error: {0}")]
    XlsxRead(#[from] calamine::XlsxError),
    #[cfg(feature = "xlsx")]
    #[error("XLSX write error: {0}")]
    XlsxWrite(#[from] rust_xlsxwriter::XlsxError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
