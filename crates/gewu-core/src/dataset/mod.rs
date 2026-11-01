//! Immutable tabular data, column schema and dataset metadata.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::{
    array::{ArrayRef, Int64Array},
    compute::cast,
    datatypes::{DataType, SchemaRef},
};

use crate::{
    column::{format, Column, ColumnCapabilities},
    file, Error, FileInfo, FileMetadata, FileType, OpenOptions, Result,
};

/// An immutable tabular dataset loaded into memory.
pub struct Dataset {
    pub(crate) source_path: PathBuf,
    pub(crate) schema: SchemaRef,
    pub(crate) columns: Vec<Column>,
    pub(crate) n_rows: usize,
    pub(crate) file_size: u64,
    pub(crate) format: FileType,
    pub(crate) metadata: FileMetadata,
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

        let columns = (0..batch.num_columns())
            .map(|i| {
                let field = schema.field(i);
                Column::new(field.name(), batch.column(i).clone())
            })
            .collect();

        Ok(Self {
            source_path,
            schema,
            columns,
            n_rows,
            file_size,
            format,
            metadata: loaded.metadata,
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
        FileInfo {
            path: self.source_path.display().to_string(),
            row_count: self.n_rows as u64,
            columns: self.columns.iter().map(Column::info).collect(),
            file_size: self.file_size,
            format: self.format,
            metadata: self.metadata.clone(),
        }
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Index of the column with this name.
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.schema
            .fields()
            .iter()
            .position(|f| f.name() == name)
            .map(|i| i as u32)
    }

    pub fn column(&self, c: u32) -> Result<&Column> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }

    pub fn column_capabilities(&self, column: u32) -> Result<ColumnCapabilities> {
        Ok(self.column(column)?.capabilities())
    }

    pub fn format_value(&self, column: u32, value: i64) -> Result<String> {
        let col = self.column(column)?;
        let data_type = col.array.data_type().clone();
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
}
