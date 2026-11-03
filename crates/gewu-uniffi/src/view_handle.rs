//! Immutable view handles and operations over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{
    AxisScale, FileType, FilterChoices, FindSession, GewuError, GewuExport, QuerySpec,
    RangeHistogram, RowBlock,
};

#[derive(uniffi::Object)]
pub struct GewuView {
    pub(crate) inner: Arc<core::View>,
}

#[uniffi::export]
impl GewuView {
    pub fn query(
        &self,
        spec: QuerySpec,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: self.inner.query(&spec.into(), &cancellation.inner)?,
        }))
    }

    pub fn reorder_columns(&self, order: Vec<u32>) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: self.inner.reorder_columns(&order)?,
        }))
    }

    pub fn columns(&self) -> Vec<u32> {
        self.inner.columns().to_vec()
    }

    pub fn row_count(&self) -> u64 {
        self.inner.row_count()
    }

    pub fn rows(&self, start: u64, count: u32, columns: Vec<u32>) -> Result<RowBlock, GewuError> {
        Ok(self.inner.rows(start, count, &columns)?.into())
    }

    pub fn export(
        &self,
        columns: Vec<u32>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        Ok(Arc::new(self.inner.export(&columns, format.into())?.into()))
    }

    pub fn nested_export_columns(&self, columns: Vec<u32>) -> Result<Vec<String>, GewuError> {
        Ok(self.inner.nested_export_columns(&columns)?)
    }

    pub fn find(&self, query: String, columns: Vec<u32>) -> Result<Arc<FindSession>, GewuError> {
        Ok(Arc::new(FindSession {
            inner: self.inner.find(&query, &columns)?,
        }))
    }

    pub fn count_nulls(
        &self,
        column: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<u64, GewuError> {
        Ok(self.inner.count_nulls(column, &cancellation.inner)?)
    }

    pub fn analyze_statistics(
        &self,
        column: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<crate::NumericStatistics, GewuError> {
        Ok(self
            .inner
            .analyze_statistics(column, &cancellation.inner)?
            .into())
    }

    pub fn analyze_distribution(
        &self,
        column: u32,
        bins: u32,
        scale: AxisScale,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<crate::Histogram, GewuError> {
        Ok(self
            .inner
            .analyze_distribution(column, bins, scale.into(), &cancellation.inner)?
            .into())
    }

    pub fn analyze_values(
        &self,
        column: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<crate::ValueSummary, GewuError> {
        Ok(self
            .inner
            .analyze_values(column, &cancellation.inner)?
            .into())
    }

    pub fn analyze_lengths(
        &self,
        column: u32,
        bins: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<crate::LengthSummary, GewuError> {
        Ok(self
            .inner
            .analyze_lengths(column, bins, &cancellation.inner)?
            .into())
    }

    pub fn filter_choices(
        &self,
        column: u32,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<FilterChoices, GewuError> {
        Ok(self
            .inner
            .filter_choices(column, &cancellation.inner)?
            .into())
    }

    pub fn filter_distribution(
        &self,
        column: u32,
        bins: u32,
        scale: AxisScale,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<Option<RangeHistogram>, GewuError> {
        Ok(self
            .inner
            .filter_distribution(column, bins, scale.into(), &cancellation.inner)?
            .map(Into::into))
    }

    pub fn analyze_scatter(
        &self,
        x_column: Option<u32>,
        y_column: u32,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
        cancellation: Arc<crate::Cancellation>,
    ) -> Result<crate::ScatterResult, GewuError> {
        Ok(self
            .inner
            .analyze_scatter(
                x_column,
                y_column,
                limit,
                x_scale.into(),
                y_scale.into(),
                &cancellation.inner,
            )?
            .into())
    }
}
