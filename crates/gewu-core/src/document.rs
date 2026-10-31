//! Immutable source data, column metadata and lazy value-index ownership.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::{ArrayRef, Int64Array},
    compute::cast,
    datatypes::{DataType, SchemaRef},
};

use crate::{
    column::{Column, FilterEditorKind},
    engine::ViewEngine,
    file, format,
    value_index::{ValueIndex, INDEX_LIMIT},
    ColumnCapabilities, ColumnInfo, DataKind, Error, FileInfo, FileMetadata, FileType, OpenOptions,
    Result,
};

mod analysis;
mod export;
mod filter;
mod query;

/// One open file. Safe to share between threads.
pub struct Document {
    source_path: PathBuf,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    n_rows: usize,
    file_size: u64,
    format: FileType,
    metadata: FileMetadata,
    indexes: Vec<OnceLock<Indexed>>,
    filter_editors: Vec<OnceLock<FilterEditorKind>>,
    views: ViewEngine,
}

/// A column's value index, built on first use.
enum Indexed {
    Built(Arc<ValueIndex>),
    /// More than [`INDEX_LIMIT`] distinct values: counted on demand instead.
    TooMany,
}

impl Document {
    /// Open a supported file using its format's default options.
    /// CSV/TSV and XLSX use the first record as headers; XLSX opens its first sheet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, &OpenOptions::default())
    }

    /// Open a file with an explicit format, header policy or XLSX sheet name.
    pub fn open_with_options(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        let source_path = path.canonicalize()?;
        let file_size = std::fs::metadata(path)?.len();
        let (format, loaded) = file::read(path, options)?;
        let batch = loaded.batch;
        let schema = batch.schema();

        let n_rows = batch.num_rows();
        if n_rows > u32::MAX as usize {
            return Err(Error::Unsupported(format!(
                "{n_rows} rows is more than the supported {}",
                u32::MAX
            )));
        }

        let columns = batch.columns().to_vec();
        let indexes = (0..columns.len()).map(|_| OnceLock::new()).collect();
        let filter_editors = (0..columns.len()).map(|_| OnceLock::new()).collect();

        Ok(Self {
            source_path,
            schema,
            columns,
            n_rows,
            file_size,
            format,
            metadata: loaded.metadata,
            indexes,
            filter_editors,
            views: ViewEngine::new(n_rows),
        })
    }

    pub fn info(&self) -> FileInfo {
        let columns = self
            .schema
            .fields()
            .iter()
            .map(|f| ColumnInfo {
                name: f.name().clone(),
                type_name: f.data_type().to_string(),
                kind: DataKind::of(f.data_type()),
                nullable: f.is_nullable(),
            })
            .collect();

        FileInfo {
            path: self.source_path.display().to_string(),
            row_count: self.n_rows as u64,
            columns,
            file_size: self.file_size,
            format: self.format,
            metadata: self.metadata.clone(),
        }
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Index of the column with this name.
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.schema
            .fields()
            .iter()
            .position(|f| f.name() == name)
            .map(|i| i as u32)
    }

    /// Display text for a raw value of a date, time, timestamp or duration
    /// column (its integer physical value, as used by histograms), formatted
    /// exactly like the table's cells. Also works for
    /// integer columns.
    pub fn format_value(&self, column: u32, value: i64) -> Result<String> {
        let data_type = self.column(column)?.data_type().clone();
        let raw: ArrayRef = Arc::new(Int64Array::from(vec![value]));
        let typed = match data_type {
            // Arrow casts these from 32-bit integers only.
            DataType::Date32 | DataType::Time32(_) => {
                cast(&cast(&raw, &DataType::Int32)?, &data_type)?
            }
            _ => cast(&raw, &data_type)?,
        };
        let f = format::formatter(typed.as_ref())?;
        Ok(f.value(0).to_string())
    }

    /// Operations supported by this column's logical type.
    pub fn column_capabilities(&self, column: u32) -> Result<ColumnCapabilities> {
        Ok(Column::new(self.column(column)?).capabilities())
    }

    // ---- internals ----

    fn column(&self, c: u32) -> Result<&ArrayRef> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }

    /// The column's value index, or `None` if it has too many values.
    fn index(&self, c: u32) -> Result<Option<Arc<ValueIndex>>> {
        let array = self.column(c)?;
        let cell = &self.indexes[c as usize];
        let found = match cell.get() {
            Some(found) => found,
            None => {
                let built = match ValueIndex::build(array, INDEX_LIMIT)? {
                    Some(idx) => Indexed::Built(Arc::new(idx)),
                    None => Indexed::TooMany,
                };
                cell.get_or_init(|| built)
            }
        };
        Ok(match found {
            Indexed::Built(idx) => Some(idx.clone()),
            Indexed::TooMany => None,
        })
    }

    /// The column's value index, for filtering by key.
    fn keyed_index(&self, c: u32) -> Result<Arc<ValueIndex>> {
        self.index(c)?.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "column {} has more than {INDEX_LIMIT} distinct values; filter it with contains or a range",
                self.schema.field(c as usize).name()
            ))
        })
    }
}
