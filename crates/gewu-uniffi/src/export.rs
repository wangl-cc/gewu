//! Independently cancellable export snapshots.

use gewu_core as core;

use crate::GewuError;

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

#[derive(uniffi::Record)]
pub struct ExportOptions {
    pub parquet_compression: ParquetCompression,
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
    /// Write the captured result. Slow: call off the main thread.
    pub fn write(&self, path: String) -> Result<(), GewuError> {
        Ok(self.inner.write(path)?)
    }

    /// Format-specific encoding options do not affect other output formats.
    pub fn write_with_options(
        &self,
        path: String,
        options: ExportOptions,
    ) -> Result<(), GewuError> {
        Ok(self.inner.write_with_options(path, &core::ExportOptions {
            parquet_compression: options.parquet_compression.into(),
        })?)
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
