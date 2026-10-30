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

use crate::{ColumnInfo, Error, Result};

#[cfg(feature = "csv")]
pub(crate) mod delimited;
#[cfg(feature = "parquet")]
mod parquet;
#[cfg(feature = "xlsx")]
mod xlsx;

#[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
pub(crate) const CHUNK_ROWS: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub file_size: u64,
    pub format: crate::FileType,
    pub metadata: crate::FileMetadata,
}

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

/// Parquet compression algorithms compiled into this build.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ParquetCompression {
    #[default]
    Zstd,
    Snappy,
    Gzip,
    Brotli,
    Lz4,
    Uncompressed,
}

/// Encoding options for an export. Format-specific options are ignored by
/// other formats; row and column selection belong to the export snapshot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    pub parquet_compression: ParquetCompression,
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

/// List XLSX worksheets in workbook order, including hidden and empty sheets.
/// Chart sheets are excluded. Does not decode worksheet cells, so Excel error
/// cells in an individual sheet do not prevent listing the others. Workbook
/// metadata, styles and shared strings are still read; this is blocking I/O.
/// Returns an empty list when the workbook has no worksheets. The path must
/// identify an XLSX file under the same detection rules as [`crate::Document::open`].
pub fn xlsx_sheet_names(path: impl AsRef<Path>) -> Result<Vec<String>> {
    FileType::Xlsx.require_enabled()?;
    let path = path.as_ref();
    let mut file = File::open(path)?;
    if FileType::detect(path, &mut file)? != FileType::Xlsx {
        return Err(Error::InvalidArgument(
            "worksheet listing requires XLSX".into(),
        ));
    }
    #[cfg(feature = "xlsx")]
    return xlsx::sheet_names(file);
    #[cfg(not(feature = "xlsx"))]
    unreachable!("XLSX availability was checked above")
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

#[cfg(any(feature = "parquet", feature = "csv"))]
impl ExportData<'_> {
    /// Borrow contiguous source rows; gather sorted/filtered rows only for the
    /// current bounded chunk. Neither codec needs a whole-column copy.
    fn batch(self, rows: &[u32]) -> Result<RecordBatch> {
        let columns = if let Some(&first) = rows
            .first()
            .filter(|_| rows.windows(2).all(|pair| pair[1] == pair[0] + 1))
        {
            self.columns
                .iter()
                .map(|array| array.slice(first as usize, rows.len()))
                .collect()
        } else {
            let indices = arrow::array::UInt32Array::from(rows.to_vec());
            self.columns
                .iter()
                .map(|array| arrow::compute::take(array.as_ref(), &indices, None))
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(RecordBatch::try_new(self.schema.clone(), columns)?)
    }
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
    options: &ExportOptions,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<()> {
    format.require_enabled()?;
    match format {
        #[cfg(feature = "parquet")]
        FileType::Parquet => parquet::write(file, options.parquet_compression, data, progress),
        #[cfg(feature = "csv")]
        FileType::Csv => delimited::write(file, b',', data, progress),
        #[cfg(feature = "csv")]
        FileType::Tsv => delimited::write(file, b'\t', data, progress),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::write(file, data, progress),
        #[allow(unreachable_patterns)]
        _ => {
            let _ = (file, options, data, progress);
            Err(Error::Unsupported(format!(
                "{format} support is disabled in this build"
            )))
        }
    }
}
