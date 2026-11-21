//! Independently cancellable export snapshots.

use std::sync::Arc;

use gewu_core::{self as core, ExportOptions, ExportProgress, ParquetCompression};

use crate::{GewuError, GewuExportedBytes};

#[uniffi::remote(Enum)]
pub enum ParquetCompression {
    Zstd,
    Snappy,
    Gzip,
    Brotli,
    Lz4,
    Uncompressed,
}

/// How to encode an export. Format-specific options do not affect other formats.
#[uniffi::remote(Record)]
pub struct ExportOptions {
    pub parquet_compression: ParquetCompression,
    /// Write nested columns to CSV, TSV and XLSX as display text instead of rejecting them.
    pub nested_as_text: bool,
    /// Start CSV and TSV output with a row of column names.
    pub include_header: bool,
}

#[uniffi::remote(Record)]
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
        Ok(self.inner.write_with_options(path, &options)?)
    }

    /// Write a CSV or TSV export into memory, failing instead of building more
    /// than `limit` bytes. Read the result through `gewu_exported_bytes`.
    pub fn write_to_memory(
        &self,
        options: ExportOptions,
        limit: u64,
    ) -> Result<Arc<GewuExportedBytes>, GewuError> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let bytes = self.inner.write_to_memory(&options, limit)?;
        Ok(Arc::new(GewuExportedBytes::new(bytes)))
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn progress(&self) -> ExportProgress {
        self.inner.progress()
    }
}

impl From<core::Export> for GewuExport {
    fn from(inner: core::Export) -> Self {
        Self { inner }
    }
}
