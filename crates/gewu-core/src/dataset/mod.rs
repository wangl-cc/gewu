//! Immutable tabular data, column schema and dataset metadata.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::datatypes::SchemaRef;

use crate::{
    column::Column, file, ColumnId, ColumnInfo, FileMetadata, FileType, OpenOptions, Result,
};

mod filter;
mod info;

pub use info::DatasetInfo;

/// An immutable tabular dataset loaded into memory.
pub struct Dataset {
    source_path: PathBuf,
    schema: SchemaRef,
    columns: Vec<Column>,
    n_rows: usize,
    file_size: u64,
    format: FileType,
    metadata: FileMetadata,
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
        Self::from_loaded(source_path, file_size, format, loaded)
    }

    fn from_loaded(
        source_path: PathBuf,
        file_size: u64,
        format: FileType,
        loaded: file::LoadedFile,
    ) -> Result<Self> {
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

    #[cfg(test)]
    pub(crate) fn from_test_batch(batch: arrow::array::RecordBatch) -> Arc<Self> {
        Arc::new(
            Self::from_loaded(PathBuf::default(), 0, FileType::Csv, file::LoadedFile {
                batch,
                metadata: FileMetadata::Delimited,
            })
            .unwrap(),
        )
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    pub fn n_rows(&self) -> usize {
        self.n_rows
    }

    /// Canonical source path, preserving the platform's native path representation.
    pub fn path(&self) -> &Path {
        &self.source_path
    }

    pub fn format(&self) -> FileType {
        self.format
    }

    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    /// Size of the file in bytes when opened.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn column_infos(&self) -> Vec<ColumnInfo> {
        self.schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, field)| ColumnInfo::from_field(ColumnId::new(i as u32), field))
            .collect()
    }

    pub fn info(&self) -> DatasetInfo {
        DatasetInfo {
            path: self.source_path.display().to_string(),
            row_count: self.n_rows as u64,
            columns: self.column_infos(),
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
