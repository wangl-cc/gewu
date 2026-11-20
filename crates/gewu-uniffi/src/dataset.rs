//! Dataset opening and metadata across the language boundary.
use std::sync::Arc;

use gewu_core::{self as core, AnalysisField, ColumnId, ColumnInfo, FileType};

use crate::{FileMetadata, GewuError, GewuOrderedView};

/// What a file contains. Mirrors Core's, with source metadata in its
/// boundary form.
#[derive(uniffi::Record)]
pub struct DatasetInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub format: FileType,
    pub file_size: u64,
    pub metadata: FileMetadata,
}

impl From<core::DatasetInfo> for DatasetInfo {
    fn from(info: core::DatasetInfo) -> Self {
        Self {
            path: info.path,
            row_count: info.row_count,
            columns: info.columns,
            format: info.format,
            file_size: info.file_size,
            metadata: info.metadata.into(),
        }
    }
}

/// How to read a file.
#[derive(uniffi::Record)]
pub struct OpenOptions {
    pub sheet: Option<String>,
    pub has_header: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            sheet: None,
            has_header: true,
        }
    }
}

#[derive(uniffi::Object)]
pub struct GewuDataset {
    pub(crate) inner: Arc<core::Dataset>,
}

#[uniffi::export]
impl GewuDataset {
    /// Read and decode a file. `options.sheet` selects an XLSX worksheet (the
    /// first one when absent); `options.has_header` applies to CSV, TSV and
    /// XLSX. Slow: call off the main thread.
    #[uniffi::constructor]
    pub fn open(path: String, options: OpenOptions) -> Result<Arc<Self>, GewuError> {
        let options = core::OpenOptions {
            sheet: options.sheet,
            has_header: options.has_header,
            ..core::OpenOptions::default()
        };
        Ok(Arc::new(Self {
            inner: Arc::new(core::Dataset::open_with_options(path, &options)?),
        }))
    }

    pub fn info(&self) -> DatasetInfo {
        self.inner.info().into()
    }

    /// Nested fields of a column, read from the schema without scanning.
    pub fn column_structure(&self, column: ColumnId) -> Result<Vec<AnalysisField>, GewuError> {
        Ok(self.inner.column_structure(column)?)
    }

    /// Every row in file order. Derive other rows with [`GewuOrderedView::apply`].
    pub fn rows(&self) -> Arc<GewuOrderedView> {
        crate::GewuFilteredView::unfiltered(Arc::clone(&self.inner)).unsorted()
    }
}
