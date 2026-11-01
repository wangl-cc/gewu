//! User viewing session combining immutable dataset and active view engine.

use std::{path::Path, sync::Arc};

use arrow::{array::ArrayRef, datatypes::SchemaRef};

use crate::{
    column::{Column, ColumnCapabilities, Dictionary},
    dataset::Dataset,
    engine::ViewEngine,
    FileInfo, OpenOptions, Result,
};

mod analysis;
mod export;
pub(crate) mod facet;
mod find;
mod query;

pub use facet::{DistinctResult, DistinctValue, FilterEditor, FilterSpec, TextFilterMode};

/// An active viewing session over an immutable dataset.
/// Holds a reference to the dataset and manages view versions, filters, and display order.
pub struct Session {
    dataset: Arc<Dataset>,
    views: ViewEngine,
}

impl Session {
    /// Open a supported file using its format's default options.
    /// CSV/TSV and XLSX use the first record as headers; XLSX opens its first sheet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dataset = Arc::new(Dataset::open(path)?);
        let n_rows = dataset.n_rows;
        Ok(Self {
            dataset,
            views: ViewEngine::new(n_rows),
        })
    }

    /// Open a file with an explicit format, header policy or XLSX sheet name.
    pub fn open_with_options(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        let dataset = Arc::new(Dataset::open_with_options(path, options)?);
        let n_rows = dataset.n_rows;
        Ok(Self {
            dataset,
            views: ViewEngine::new(n_rows),
        })
    }

    /// Create a viewing session from an existing in-memory dataset.
    pub fn from_dataset(dataset: Arc<Dataset>) -> Self {
        let n_rows = dataset.n_rows;
        Self {
            dataset,
            views: ViewEngine::new(n_rows),
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

    pub(crate) fn dictionary(&self, c: u32) -> Result<Option<Arc<Dictionary>>> {
        Ok(self.column(c)?.dictionary().cloned())
    }

    pub(crate) fn keyed_dictionary(&self, c: u32) -> Result<Arc<Dictionary>> {
        let col = self.column(c)?;
        col.dictionary().cloned().ok_or_else(|| {
            crate::Error::InvalidArgument(format!(
                "column {} has more than {} non-null distinct values; filter it with contains or a range",
                col.name,
                crate::column::MAX_DISTINCT_VALUES,
            ))
        })
    }
}
