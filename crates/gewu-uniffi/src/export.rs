//! Independently cancellable export snapshots.

use std::sync::Arc;

use gewu_core as core;

use crate::{ExportedBytes, GewuError};

#[derive(Clone, Copy, uniffi::Enum)]
pub enum ParquetCompression {
    Zstd,
    Snappy,
    Gzip,
    Brotli,
    Lz4,
    Uncompressed,
}

impl From<ParquetCompression> for core::ParquetCompression {
    fn from(value: ParquetCompression) -> Self {
        match value {
            ParquetCompression::Zstd => Self::Zstd,
            ParquetCompression::Snappy => Self::Snappy,
            ParquetCompression::Gzip => Self::Gzip,
            ParquetCompression::Brotli => Self::Brotli,
            ParquetCompression::Lz4 => Self::Lz4,
            ParquetCompression::Uncompressed => Self::Uncompressed,
        }
    }
}

/// How to encode an export. Format-specific options do not affect other formats.
#[derive(uniffi::Record)]
pub struct ExportOptions {
    pub parquet_compression: ParquetCompression,
    /// Write nested columns to CSV, TSV and XLSX as display text instead of rejecting them.
    pub nested_as_text: bool,
    /// Start CSV and TSV output with a row of column names.
    pub include_header: bool,
}

impl From<ExportOptions> for core::ExportOptions {
    fn from(options: ExportOptions) -> Self {
        Self {
            parquet_compression: options.parquet_compression.into(),
            nested_as_text: options.nested_as_text,
            include_header: options.include_header,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ExportProgress {
    pub rows_written: u64,
    pub total_rows: u64,
}

/// An immutable view snapshot with independent cancellation. Single-use.
#[derive(uniffi::Object)]
pub struct GewuExport {
    inner: core::Export,
}

#[uniffi::export]
impl GewuExport {
    /// Write the captured rows to `path`, replacing it only on success. Slow:
    /// call off the main thread.
    pub fn write(&self, path: String, options: ExportOptions) -> Result<(), GewuError> {
        Ok(self.inner.write_with_options(path, &options.into())?)
    }

    /// Write a CSV or TSV export into memory, failing instead of building more
    /// than `limit` bytes. Read the result through `gewu_exported_bytes`.
    pub fn write_to_memory(
        &self,
        options: ExportOptions,
        limit: u64,
    ) -> Result<Arc<ExportedBytes>, GewuError> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let bytes = self.inner.write_to_memory(&options.into(), limit)?;
        Ok(Arc::new(ExportedBytes::new(bytes)))
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn progress(&self) -> ExportProgress {
        let progress = self.inner.progress();
        ExportProgress {
            rows_written: progress.rows_written,
            total_rows: progress.total_rows,
        }
    }
}

impl From<core::Export> for GewuExport {
    fn from(inner: core::Export) -> Self {
        Self { inner }
    }
}
