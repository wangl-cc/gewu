//! User viewing session combining immutable dataset and active view engine.

use std::{
    path::Path,
    sync::{Arc, OnceLock},
};

use arrow::{array::ArrayRef, datatypes::SchemaRef};

use crate::{
    column::{Column, ColumnCapabilities, ValueIndex},
    dataset::Dataset,
    engine::ViewEngine,
    FileInfo, OpenOptions, Result,
};

mod analysis;
mod export;
mod facet;
mod find;
mod query;

pub use facet::{FilterEditor, FilterSpec, TextFilterMode};

/// An active viewing session over an immutable dataset.
/// Holds a reference to the dataset and manages view versions, filters, and display order.
pub struct Session {
    dataset: Arc<Dataset>,
    views: ViewEngine,
    facet_indices: Vec<OnceLock<Option<Arc<ValueIndex>>>>,
}

impl Session {
    /// Open a supported file using its format's default options.
    /// CSV/TSV and XLSX use the first record as headers; XLSX opens its first sheet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dataset = Arc::new(Dataset::open(path)?);
        let n_rows = dataset.n_rows;
        let n_cols = dataset.columns().len();
        Ok(Self {
            dataset,
            views: ViewEngine::new(n_rows),
            facet_indices: (0..n_cols).map(|_| OnceLock::new()).collect(),
        })
    }

    /// Open a file with an explicit format, header policy or XLSX sheet name.
    pub fn open_with_options(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        let dataset = Arc::new(Dataset::open_with_options(path, options)?);
        let n_rows = dataset.n_rows;
        let n_cols = dataset.columns().len();
        Ok(Self {
            dataset,
            views: ViewEngine::new(n_rows),
            facet_indices: (0..n_cols).map(|_| OnceLock::new()).collect(),
        })
    }

    /// Create a viewing session from an existing in-memory dataset.
    pub fn from_dataset(dataset: Arc<Dataset>) -> Self {
        let n_rows = dataset.n_rows;
        let n_cols = dataset.columns().len();
        Self {
            dataset,
            views: ViewEngine::new(n_rows),
            facet_indices: (0..n_cols).map(|_| OnceLock::new()).collect(),
        }
    }

    /// Access the underlying immutable dataset.
    pub fn dataset(&self) -> &Arc<Dataset> {
        &self.dataset
    }

    pub fn info(&self) -> FileInfo {
        self.dataset.info()
    }

    pub fn schema(&self) -> &SchemaRef {
        self.dataset.schema()
    }

    /// Index of the column with this name.
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.dataset.column_index(name)
    }

    /// Display text for a raw value of a date, time, timestamp or duration
    /// column (its integer physical value, as used by histograms), formatted
    /// exactly like the table's cells. Also works for integer columns.
    pub fn format_value(&self, column: u32, value: i64) -> Result<String> {
        self.dataset.format_value(column, value)
    }

    /// Operations supported by this column's logical type.
    pub fn column_capabilities(&self, column: u32) -> Result<ColumnCapabilities> {
        self.dataset.column_capabilities(column)
    }

    // ---- internal helpers for session submodules ----

    #[inline]
    pub fn n_rows(&self) -> usize {
        self.dataset.n_rows
    }

    #[inline]
    pub(crate) fn column(&self, c: u32) -> Result<&Column> {
        self.dataset.column(c)
    }

    #[inline]
    pub(crate) fn column_array(&self, c: u32) -> Result<&ArrayRef> {
        Ok(&self.dataset.column(c)?.array)
    }

    pub(crate) fn index(&self, c: u32) -> Result<Option<Arc<ValueIndex>>> {
        let col = self.column(c)?;
        let lock = self.facet_indices.get(c as usize).ok_or_else(|| {
            crate::Error::InvalidArgument(format!("column index {c} out of bounds"))
        })?;
        if let Some(cached) = lock.get() {
            return Ok(cached.clone());
        }
        let built = ValueIndex::build(&col.array, crate::column::INDEX_LIMIT)?.map(Arc::new);
        let _ = lock.set(built.clone());
        Ok(built)
    }

    pub(crate) fn keyed_index(&self, c: u32) -> Result<Arc<ValueIndex>> {
        let col_name = self.column(c)?.name.clone();
        self.index(c)?.ok_or_else(|| {
            crate::Error::InvalidArgument(format!(
                "column {col_name} has more than {} distinct values; filter it with contains or a range",
                crate::column::INDEX_LIMIT,
            ))
        })
    }
}
