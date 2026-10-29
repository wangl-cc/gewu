//! File adapters translate external formats to and from Arrow data.
//! Document owns decoded data; Export owns snapshots and destination commits.

#[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
use std::sync::atomic::Ordering;
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64},
};

use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::SchemaRef,
};

use crate::{Error, Result};

#[cfg(feature = "csv")]
pub(crate) mod delimited;
#[cfg(feature = "parquet")]
mod parquet;
#[cfg(feature = "xlsx")]
mod xlsx;

#[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
pub(crate) const CHUNK_ROWS: usize = 8 * 1024;

/// A recognized file format. Variants remain available when a codec feature
/// is disabled; attempting to use that codec returns an Unsupported error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Parquet,
    Csv,
    Tsv,
    Xlsx,
}

impl FileType {
    pub fn is_enabled(self) -> bool {
        match self {
            Self::Parquet => cfg!(feature = "parquet"),
            Self::Csv | Self::Tsv => cfg!(feature = "csv"),
            Self::Xlsx => cfg!(feature = "xlsx"),
        }
    }

    pub(crate) fn require_enabled(self) -> Result<()> {
        if self.is_enabled() {
            Ok(())
        } else {
            Err(Error::Unsupported(format!(
                "{self} support is disabled in this build"
            )))
        }
    }

    fn detect(path: &Path, file: &mut File) -> Result<Self> {
        // A CSV header can itself start with PAR1, so a recognized extension
        // takes precedence over the extensionless Parquet fallback.
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("parquet") => return Ok(Self::Parquet),
            Some("csv") => return Ok(Self::Csv),
            Some("tsv") => return Ok(Self::Tsv),
            Some("xlsx") => return Ok(Self::Xlsx),
            _ => {}
        }
        let mut magic = [0; 4];
        let n = file.read(&mut magic)?;
        file.rewind()?;
        if n == 4 && &magic == b"PAR1" {
            return Ok(Self::Parquet);
        }
        Err(Error::Unsupported(
            "expected a Parquet, CSV, TSV or XLSX file; specify a format for other extensions"
                .into(),
        ))
    }
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Parquet => "Parquet",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
            Self::Xlsx => "XLSX",
        })
    }
}

/// Options for opening a table. CSV/TSV infer types over the entire input.
/// XLSX opens its first worksheet unless a sheet name is supplied.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub format: Option<FileType>,
    /// Treat the first record as column names for CSV, TSV and XLSX.
    pub has_header: bool,
    /// XLSX worksheet name. Rejected for other file formats.
    pub sheet: Option<String>,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            format: None,
            has_header: true,
            sheet: None,
        }
    }
}

/// Metadata that belongs to a particular file format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileMetadata {
    Parquet {
        row_groups: u32,
        compression: Vec<String>,
        created_by: Option<String>,
        key_values: Vec<(String, String)>,
    },
    Delimited,
    Xlsx {
        sheet: String,
        sheets: Vec<String>,
    },
}

pub(crate) struct LoadedFile {
    pub batch: RecordBatch,
    pub metadata: FileMetadata,
}

pub(crate) fn read(path: &Path, options: &OpenOptions) -> Result<(FileType, LoadedFile)> {
    let mut file = File::open(path)?;
    let format = match options.format {
        Some(format) => format,
        None => FileType::detect(path, &mut file)?,
    };
    format.require_enabled()?;
    if options.sheet.is_some() && format != FileType::Xlsx {
        return Err(Error::InvalidArgument(
            "worksheet selection requires XLSX".into(),
        ));
    }
    let loaded = match format {
        #[cfg(feature = "parquet")]
        FileType::Parquet => parquet::read(file),
        #[cfg(feature = "csv")]
        FileType::Csv => delimited::read(file, b',', options),
        #[cfg(feature = "csv")]
        FileType::Tsv => delimited::read(file, b'\t', options),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::read(file, options),
        #[allow(unreachable_patterns)]
        _ => Err(Error::Unsupported(format!(
            "{format} support is disabled in this build"
        ))),
    };
    loaded.map(|loaded| (format, loaded))
}

/// Shared table materialization for batch-oriented readers.
#[cfg(any(feature = "parquet", feature = "csv"))]
fn collect_batches(
    schema: &SchemaRef,
    batches: impl Iterator<Item = std::result::Result<RecordBatch, arrow::error::ArrowError>>,
) -> Result<RecordBatch> {
    let mut kept = Vec::new();
    let mut rows = 0usize;
    for batch in batches {
        let batch = batch?;
        rows = rows
            .checked_add(batch.num_rows())
            .ok_or_else(|| Error::Unsupported("too many rows".into()))?;
        if rows > u32::MAX as usize {
            return Err(Error::Unsupported(format!("more than {} rows", u32::MAX)));
        }
        kept.push(batch);
    }
    Ok(arrow::compute::concat_batches(schema, &kept)?)
}

/// Borrowed export snapshot. Codecs decide whether to access source rows
/// directly or gather them into batches; no mandatory intermediate copy.
#[derive(Clone, Copy)]
#[cfg_attr(
    not(any(feature = "parquet", feature = "csv", feature = "xlsx")),
    expect(
        dead_code,
        reason = "No codec consumes the export snapshot in this configuration"
    )
)]
pub(crate) struct ExportData<'a> {
    pub schema: &'a SchemaRef,
    pub columns: &'a [ArrayRef],
    pub rows: &'a [u32],
}

#[derive(Clone, Copy)]
#[cfg_attr(
    not(any(feature = "parquet", feature = "csv", feature = "xlsx")),
    expect(dead_code, reason = "No codec reports progress in this configuration")
)]
pub(crate) struct WriteProgress<'a> {
    pub cancelled: &'a AtomicBool,
    pub rows_written: &'a AtomicU64,
}

#[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
impl WriteProgress<'_> {
    pub fn check(self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn advance(self, rows: usize) {
        self.rows_written.fetch_add(rows as u64, Ordering::Relaxed);
    }
}

pub(crate) fn write(
    file: &mut File,
    format: FileType,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<()> {
    format.require_enabled()?;
    match format {
        #[cfg(feature = "parquet")]
        FileType::Parquet => parquet::write(file, data, progress),
        #[cfg(feature = "csv")]
        FileType::Csv => delimited::write(file, b',', data, progress),
        #[cfg(feature = "csv")]
        FileType::Tsv => delimited::write(file, b'\t', data, progress),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::write(file, data, progress),
        #[allow(unreachable_patterns)]
        _ => {
            let _ = (file, data, progress);
            Err(Error::Unsupported(format!(
                "{format} support is disabled in this build"
            )))
        }
    }
}
