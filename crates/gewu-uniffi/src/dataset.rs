//! Dataset opening and metadata across the language boundary.
use std::sync::Arc;

use gewu_core as core;

use crate::{AnalysisCapabilities, ColumnInfo, FileMetadata, FileType, GewuError};

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
            columns: info.columns.into_iter().map(Into::into).collect(),
            format: info.format.into(),
            file_size: info.file_size,
            metadata: info.metadata.into(),
        }
    }
}

#[derive(uniffi::Object)]
pub struct GewuDataset {
    inner: Arc<core::Dataset>,
}

#[uniffi::export]
impl GewuDataset {
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: Arc::new(core::Dataset::open(path)?),
        }))
    }

    #[uniffi::constructor]
    pub fn open_with_options(
        path: String,
        sheet: Option<String>,
        has_header: bool,
    ) -> Result<Arc<Self>, GewuError> {
        let options = core::OpenOptions {
            sheet,
            has_header,
            ..core::OpenOptions::default()
        };
        Ok(Arc::new(Self {
            inner: Arc::new(core::Dataset::open_with_options(path, &options)?),
        }))
    }

    #[uniffi::constructor]
    pub fn open_sheet(path: String, sheet: String) -> Result<Arc<Self>, GewuError> {
        let options = core::OpenOptions {
            sheet: Some(sheet),
            ..core::OpenOptions::default()
        };
        Ok(Arc::new(Self {
            inner: Arc::new(core::Dataset::open_with_options(path, &options)?),
        }))
    }

    pub fn query(
        &self,
        spec: crate::QuerySpec,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<Arc<crate::GewuView>, GewuError> {
        Ok(Arc::new(crate::GewuView {
            inner: self.inner.query(&spec.into(), &cancellation.inner)?,
        }))
    }

    pub fn info(&self) -> DatasetInfo {
        self.inner.info().into()
    }

    pub fn analysis_capabilities(&self, column: u32) -> Result<AnalysisCapabilities, GewuError> {
        Ok(self.inner.analysis_capabilities(column)?.into())
    }

    pub fn column_structure(&self, column: u32) -> Result<Vec<crate::AnalysisField>, GewuError> {
        Ok(self
            .inner
            .column_structure(column)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn view(&self) -> Arc<crate::GewuView> {
        Arc::new(crate::GewuView {
            inner: self.inner.view(),
        })
    }
}
