//! Immutable tabular data, column schema and dataset metadata.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::datatypes::SchemaRef;

use crate::{column::Column, file, Error, FileInfo, FileMetadata, FileType, OpenOptions, Result};

mod query;

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
    /// A full-data initial view. Existing views remain valid independently of this handle.
    pub fn view(self: &Arc<Self>) -> Arc<crate::View> {
        crate::View::all(self.clone())
    }

    pub fn analysis_capabilities(&self, column: u32) -> Result<crate::AnalysisCapabilities> {
        Ok(crate::stats::capabilities(
            self.column(column)?.array().data_type(),
        ))
    }

    pub fn column_structure(&self, column: u32) -> Result<Vec<crate::AnalysisField>> {
        crate::stats::analysis::structure(self.column(column)?.array().data_type())
    }

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
            .collect::<Result<Vec<_>>>()?;

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

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    pub fn n_rows(&self) -> usize {
        self.n_rows
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

    pub(crate) fn columns(&self) -> &[Column] {
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

    pub(crate) fn column(&self, c: u32) -> Result<&Column> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }
}
