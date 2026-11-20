//! Immutable tabular data, column schema and dataset metadata.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::datatypes::SchemaRef;

use crate::{column::Column, file, ColumnId, FileMetadata, FileType, OpenOptions, Result};

mod filter;
mod info;

pub use info::DatasetInfo;

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
    /// A full-data unfiltered view. Existing views remain valid independently of this handle.
    pub fn unfiltered(self: &Arc<Self>) -> Arc<crate::FilteredView> {
        crate::FilteredView::all(self.clone())
    }

    pub fn column_structure(&self, column: ColumnId) -> Result<Vec<crate::AnalysisField>> {
        Ok(crate::stats::analysis::structure(
            self.column(column)?.array().data_type(),
        )?)
    }

    /// Open a supported file using its format's default options.
    /// CSV/TSV and XLSX use the first record as headers; XLSX opens its first sheet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, &OpenOptions::default())
    }

    /// Open a file with an explicit format, header policy or XLSX sheet name.
    pub fn open_with_options(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        let source_path = path
            .canonicalize()
            .map_err(|error| crate::IoError::new(crate::IoOperation::Canonicalize, path, error))?;
        let file_size = std::fs::metadata(path)
            .map_err(|error| crate::IoError::new(crate::IoOperation::ReadMetadata, path, error))?
            .len();
        let (format, loaded) = file::read(path, options)?;
        let batch = loaded.batch;
        let schema = batch.schema();

        let n_rows = batch.num_rows();
        if n_rows > u32::MAX as usize {
            return Err(crate::FileError::DatasetRows {
                actual: n_rows as u64,
                limit: u32::MAX as u64,
            }
            .into());
        }

        let columns = (0..batch.num_columns())
            .map(|i| {
                let field = schema.field(i);
                Column::new(field.name(), batch.column(i).clone())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

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

    pub fn info(&self) -> DatasetInfo {
        DatasetInfo {
            path: self.source_path.display().to_string(),
            row_count: self.n_rows as u64,
            columns: self
                .columns
                .iter()
                .enumerate()
                .map(|(i, col)| col.info(ColumnId::new(i as u32)))
                .collect(),
            file_size: self.file_size,
            format: self.format,
            metadata: self.metadata.clone(),
        }
    }

    /// Identifier of the column with this name.
    pub fn column_id(&self, name: &str) -> Option<ColumnId> {
        self.schema
            .fields()
            .iter()
            .position(|f| f.name() == name)
            .map(|i| ColumnId::new(i as u32))
    }

    pub(crate) fn column(&self, c: ColumnId) -> Result<&Column, crate::ColumnError> {
        self.columns
            .get(c.index())
            .ok_or_else(|| crate::ColumnError::NotFound {
                column: c,
                column_count: self.columns.len() as u32,
            })
    }
}
