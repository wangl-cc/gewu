//! File adapters translate external formats to and from Arrow data.
//! Dataset owns decoded data; Export owns snapshots and destination commits.

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

#[cfg(any(feature = "parquet", feature = "csv"))]
use crate::row_ids::RowWindow;
use crate::{row_ids::RowSlice, Result};

#[cfg(feature = "csv")]
pub(crate) mod delimited;
mod error;
pub mod export;
#[cfg(feature = "parquet")]
mod parquet;
#[cfg(feature = "xlsx")]
mod xlsx;

pub use error::{CodecError, ExportError, FileError, FileOperation};
pub use export::{Export, ExportProgress};

#[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
pub(crate) const CHUNK_ROWS: usize = 8 * 1024;

/// A recognized file format. Variants remain available when a codec feature
/// is disabled; attempting to use that codec returns `FileError::FormatDisabled`.
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

    pub(crate) fn require_enabled(self) -> Result<(), FileError> {
        if self.is_enabled() {
            Ok(())
        } else {
            Err(crate::FileError::FormatDisabled { format: self })
        }
    }

    /// The format a path's extension names, ignoring case. The content is not inspected,
    /// so an extensionless Parquet file has none; opening it still detects the format.
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        [Self::Parquet, Self::Csv, Self::Tsv, Self::Xlsx]
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }

    /// The conventional file extension, lowercase and without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Parquet => "parquet",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Xlsx => "xlsx",
        }
    }

    /// Whether the first row can be read either as column names or as data
    /// ([`OpenOptions::has_header`]).
    pub fn has_header_choice(self) -> bool {
        matches!(self, Self::Csv | Self::Tsv | Self::Xlsx)
    }

    /// Whether a file holds named worksheets ([`OpenOptions::sheet`]).
    pub fn has_worksheets(self) -> bool {
        self == Self::Xlsx
    }

    fn detect(path: &Path, file: &mut File) -> Result<Self, FileError> {
        // A CSV header can itself start with PAR1, so a recognized extension
        // takes precedence over the extensionless Parquet fallback.
        if let Some(format) = Self::from_path(path) {
            return Ok(format);
        }
        let mut magic = [0; 4];
        let n = file
            .read(&mut magic)
            .map_err(|error| crate::IoError::new(crate::IoOperation::Read, path, error))?;
        file.rewind()
            .map_err(|error| crate::IoError::new(crate::IoOperation::Seek, path, error))?;
        if n == 4 && &magic == b"PAR1" {
            return Ok(Self::Parquet);
        }
        Err(crate::FileError::UnknownFormat)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    pub parquet_compression: ParquetCompression,
    /// Write nested columns to CSV, TSV and XLSX as their display text, such
    /// as `[a, b]`, instead of rejecting them. Parquet always keeps their structure.
    pub nested_as_text: bool,
    /// Start CSV and TSV output with a row of column names.
    pub include_header: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            parquet_compression: ParquetCompression::default(),
            nested_as_text: false,
            include_header: true,
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

/// List XLSX worksheets in workbook order, including hidden and empty sheets.
/// Chart sheets are excluded. Does not decode worksheet cells, so Excel error
/// cells in an individual sheet do not prevent listing the others. Workbook
/// metadata, styles and shared strings are still read; this is blocking I/O.
/// Returns an empty list when the workbook has no worksheets. The path must
/// identify an XLSX file under the same detection rules as [`crate::Dataset::open`].
pub fn xlsx_sheet_names(path: impl AsRef<Path>) -> Result<Vec<String>> {
    FileType::Xlsx.require_enabled()?;
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|error| crate::IoError::new(crate::IoOperation::Open, path, error))?;
    let actual = FileType::detect(path, &mut file)?;
    if actual != FileType::Xlsx {
        return Err(crate::FileError::UnexpectedFormat {
            operation: crate::FileOperation::SheetListing,
            expected: FileType::Xlsx,
            actual,
        }
        .into());
    }
    #[cfg(feature = "xlsx")]
    return Ok(xlsx::sheet_names(file).map_err(|error| FileError::decode(FileType::Xlsx, error))?);
    #[cfg(not(feature = "xlsx"))]
    unreachable!("XLSX availability was checked above")
}

pub(crate) fn read(
    path: &Path,
    options: &OpenOptions,
) -> Result<(FileType, LoadedFile), FileError> {
    let mut file = File::open(path)
        .map_err(|error| crate::IoError::new(crate::IoOperation::Open, path, error))?;
    let format = match options.format {
        Some(format) => format,
        None => FileType::detect(path, &mut file)?,
    };
    format.require_enabled()?;
    if options.sheet.is_some() && format != FileType::Xlsx {
        return Err(crate::FileError::UnexpectedFormat {
            operation: crate::FileOperation::SheetSelection,
            expected: FileType::Xlsx,
            actual: format,
        });
    }
    let loaded = match format {
        #[cfg(feature = "parquet")]
        FileType::Parquet => parquet::read(file),
        #[cfg(feature = "csv")]
        FileType::Csv => delimited::read(file, path, b',', options),
        #[cfg(feature = "csv")]
        FileType::Tsv => delimited::read(file, path, b'\t', options),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::read(file, options),
        #[allow(unreachable_patterns)]
        _ => Err(crate::FileError::FormatDisabled { format }),
    };
    loaded
        .map_err(|error| FileError::decode(format, error))
        .map(|loaded| (format, loaded))
}

/// Shared table materialization for batch-oriented readers.
#[cfg(any(feature = "parquet", feature = "csv"))]
fn collect_batches(
    schema: &SchemaRef,
    batches: impl Iterator<Item = std::result::Result<RecordBatch, arrow::error::ArrowError>>,
) -> Result<RecordBatch, FileError> {
    let mut kept = Vec::new();
    let mut rows = 0usize;
    for batch in batches {
        let batch = batch?;
        rows = rows.saturating_add(batch.num_rows());
        if rows > u32::MAX as usize {
            return Err(crate::FileError::DatasetRows {
                actual: rows as u64,
                limit: u32::MAX as u64,
            });
        }
        kept.push(batch);
    }
    if kept.len() == 1 {
        return Ok(kept.pop().expect("one collected batch"));
    }
    Ok(arrow::compute::concat_batches(schema, &kept)?)
}

/// Borrowed export snapshot. Codecs decide whether to access source rows
/// directly or gather them into batches; no mandatory intermediate copy.
#[derive(Clone)]
#[cfg_attr(
    not(any(feature = "parquet", feature = "csv", feature = "xlsx")),
    expect(
        dead_code,
        reason = "No codec consumes the export snapshot in this configuration"
    )
)]
pub(crate) struct ExportData<'a> {
    /// The written schema: nested columns written as text appear as `Utf8`.
    pub schema: SchemaRef,
    pub columns: &'a [ArrayRef],
    pub rows: RowSlice<'a>,
    /// One flag per column: write its display text instead of its values.
    pub as_text: &'a [bool],
}

#[cfg(any(feature = "parquet", feature = "csv"))]
impl ExportData<'_> {
    /// Borrow contiguous source rows; gather sorted/filtered rows only for the
    /// current bounded chunk. Neither codec needs a whole-column copy. Columns
    /// written as text are formatted for this chunk only.
    fn batch(&self, rows: RowWindow<'_>) -> Result<RecordBatch, crate::InternalError> {
        let columns: Vec<ArrayRef> = if let Some(range) = rows.contiguous_range() {
            self.columns
                .iter()
                .map(|array| array.slice(range.start, range.len()))
                .collect()
        } else {
            let indices = arrow::array::UInt32Array::from_iter_values(rows.iter_u32());
            self.columns
                .iter()
                .map(|array| arrow::compute::take(array.as_ref(), &indices, None))
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let columns = columns
            .into_iter()
            .zip(self.as_text)
            .map(|(array, &as_text)| {
                if as_text {
                    display_text(array.as_ref())
                } else {
                    Ok(array)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RecordBatch::try_new(self.schema.clone(), columns)?)
    }
}

/// The display text of every value, keeping nulls.
#[cfg(any(feature = "parquet", feature = "csv"))]
fn display_text(array: &dyn arrow::array::Array) -> Result<ArrayRef, crate::InternalError> {
    use std::fmt::Write as _;
    let formatter = crate::format::formatter(array)?;
    let mut builder = arrow::array::StringBuilder::with_capacity(array.len(), array.len() * 8);
    let mut text = String::new();
    for row in 0..array.len() {
        if array.is_null(row) {
            builder.append_null();
        } else {
            text.clear();
            write!(text, "{}", formatter.value(row)).expect("formatting into String succeeds");
            builder.append_value(&text);
        }
    }
    Ok(std::sync::Arc::new(builder.finish()))
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
    pub fn check(self) -> Result<(), ExportError> {
        if self.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn advance(self, rows: usize) {
        self.rows_written
            .fetch_add(rows as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) fn write(
    file: &mut File,
    format: FileType,
    options: &ExportOptions,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<(), ExportError> {
    if !format.is_enabled() {
        return Err(ExportError::FormatDisabled { format });
    }
    let result = match format {
        #[cfg(feature = "parquet")]
        FileType::Parquet => parquet::write(file, options.parquet_compression, data, progress),
        #[cfg(feature = "csv")]
        FileType::Csv => delimited::write(file, b',', options.include_header, data, progress),
        #[cfg(feature = "csv")]
        FileType::Tsv => delimited::write(file, b'\t', options.include_header, data, progress),
        #[cfg(feature = "xlsx")]
        FileType::Xlsx => xlsx::write(file, data, progress),
        #[allow(unreachable_patterns)]
        _ => {
            let _ = (file, options, data, progress);
            Err(ExportError::FormatDisabled { format })
        }
    };
    result.map_err(|error| ExportError::encode(format, error))
}

/// Write a CSV or TSV export into memory, failing with
/// [`ExportError::SizeLimit`] as soon as it would exceed `limit` bytes.
pub(crate) fn write_delimited_to_memory(
    format: FileType,
    options: &ExportOptions,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
    limit: usize,
) -> Result<Vec<u8>, ExportError> {
    let delimiter = match format {
        FileType::Csv => b',',
        FileType::Tsv => b'\t',
        other => {
            return Err(crate::InternalError::Invariant(format!(
                "{other} exports cannot be written to memory"
            ))
            .into())
        }
    };
    #[cfg(not(feature = "csv"))]
    {
        let _ = (delimiter, options, data, progress, limit);
        Err(ExportError::FormatDisabled { format })
    }
    #[cfg(feature = "csv")]
    {
        let mut buffer = LimitedBuffer {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        };
        let result = delimited::write(
            &mut buffer,
            delimiter,
            options.include_header,
            data,
            progress,
        );

        if buffer.exceeded {
            return Err(ExportError::SizeLimit {
                limit: limit as u64,
            });
        }
        result.map_err(|error| ExportError::encode(format, error))?;
        Ok(buffer.bytes)
    }
}
/// An in-memory destination that refuses to grow past its limit.
#[cfg(feature = "csv")]
struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

#[cfg(feature = "csv")]
impl std::io::Write for LimitedBuffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() + data.len() > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::other("export exceeds its size limit"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, any(feature = "parquet", feature = "csv")))]
mod tests;
