//! Dataset opening and metadata across the language boundary.
use std::sync::Arc;

use gewu_core as core;

use crate::{AnalysisSection, FileInfo, Filter, GewuError};

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

    pub fn info(&self) -> FileInfo {
        self.inner.as_ref().into()
    }

    pub fn analysis_capabilities(&self, column: u32) -> Result<Vec<AnalysisSection>, GewuError> {
        Ok(self
            .inner
            .analysis_capabilities(column)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn column_structure(&self, column: u32) -> Result<Vec<crate::AnalysisField>, GewuError> {
        Ok(self
            .inner
            .column_structure(column)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn format_value(&self, column: u32, value: i64) -> Result<String, GewuError> {
        Ok(self.inner.format_value(column, value)?)
    }

    pub fn validate_filter(
        &self,
        column: u32,
        filter: Filter,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<(), GewuError> {
        Ok(self
            .inner
            .validate_filter(column, &filter.into(), &cancellation.inner)?)
    }

    pub fn view(&self) -> Arc<crate::GewuView> {
        Arc::new(crate::GewuView {
            inner: self.inner.view(),
        })
    }
}
