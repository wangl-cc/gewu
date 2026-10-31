//! Immutable tabular data, column schema and lazy per-column indexes.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::{ArrayRef, Int64Array},
    compute::cast,
    datatypes::{DataType, SchemaRef},
};

pub(crate) mod format;
pub(crate) mod kind;
pub(crate) mod scale;
pub(crate) mod value_index;

pub use kind::LogicalType;
pub use scale::AxisScale;
pub use value_index::{DistinctResult, DistinctValue};

use crate::{
    column::{Column, FilterEditorKind, CHOICE_LIMIT},
    file,
    value_index::{ValueIndex, INDEX_LIMIT},
    ColumnCapabilities, ColumnInfo, Error, FileInfo, FileMetadata, FileType, OpenOptions, Result,
};

/// A column's value index, built on first use.
pub(crate) enum Indexed {
    Built(Arc<ValueIndex>),
    /// More than [`INDEX_LIMIT`] distinct values: counted on demand instead.
    TooMany,
}

/// An immutable tabular dataset loaded into memory.
pub struct Dataset {
    pub(crate) source_path: PathBuf,
    pub(crate) schema: SchemaRef,
    pub(crate) columns: Vec<ArrayRef>,
    pub(crate) n_rows: usize,
    pub(crate) file_size: u64,
    pub(crate) format: FileType,
    pub(crate) metadata: FileMetadata,
    pub(crate) indexes: Vec<OnceLock<Indexed>>,
    pub(crate) filter_editors: Vec<OnceLock<FilterEditorKind>>,
}

impl Dataset {
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
        })
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    pub fn n_rows(&self) -> usize {
        self.n_rows
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn format(&self) -> FileType {
        self.format
    }

    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    pub fn info(&self) -> FileInfo {
        let columns = self
            .schema
            .fields()
            .iter()
            .map(|f| {
                let logical_type = LogicalType::from_arrow(f.data_type());
                ColumnInfo {
                    name: f.name().clone(),
                    display_type: logical_type.display_name(),
                    technical_type: f.data_type().to_string(),
                    logical_type,
                    nullable: f.is_nullable(),
                }
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

    /// Index of the column with this name.
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.schema
            .fields()
            .iter()
            .position(|f| f.name() == name)
            .map(|i| i as u32)
    }

    pub fn column(&self, c: u32) -> Result<&ArrayRef> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }

    pub fn column_capabilities(&self, column: u32) -> Result<ColumnCapabilities> {
        Ok(Column::new(self.column(column)?).capabilities())
    }

    pub fn format_value(&self, column: u32, value: i64) -> Result<String> {
        let data_type = self.column(column)?.data_type().clone();
        let raw: ArrayRef = Arc::new(Int64Array::from(vec![value]));
        let typed = match data_type {
            DataType::Date32 | DataType::Time32(_) => {
                cast(&cast(&raw, &DataType::Int32)?, &data_type)?
            }
            _ => cast(&raw, &data_type)?,
        };
        let f = format::formatter(typed.as_ref())?;
        Ok(f.value(0).to_string())
    }

    /// The column's value index, or `None` if it has too many values.
    pub(crate) fn index(&self, c: u32) -> Result<Option<Arc<ValueIndex>>> {
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
    pub(crate) fn keyed_index(&self, c: u32) -> Result<Arc<ValueIndex>> {
        self.index(c)?.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "column {} has more than {INDEX_LIMIT} distinct values; filter it with contains or a range",
                self.schema.field(c as usize).name()
            ))
        })
    }

    pub(crate) fn filter_editor_kind(&self, column: u32) -> Result<FilterEditorKind> {
        let cell = &self.filter_editors[column as usize];
        if let Some(kind) = cell.get() {
            return Ok(*kind);
        }
        let array = self.column(column)?;
        let typed = Column::new(array);
        let supported = arrow::row::RowConverter::supports_fields(&[arrow::row::SortField::new(
            array.data_type().clone(),
        )]);
        let low_cardinality = if !supported {
            false
        } else {
            match self.indexes[column as usize].get() {
                Some(Indexed::Built(index)) => index.len() <= CHOICE_LIMIT,
                Some(Indexed::TooMany) => false,
                None => crate::value_index::has_at_most(array, CHOICE_LIMIT)?,
            }
        };
        Ok(*cell.get_or_init(|| typed.filter_editor(low_cardinality)))
    }
}
