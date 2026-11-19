//! Immutable filtered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{
    AxisScale, ColumnId, ColumnSort, FilterChoices, GewuError, GewuOrderedView, RangeHistogram,
};

#[derive(uniffi::Object)]
pub struct GewuFilteredView {
    pub(crate) inner: Arc<core::FilteredView>,
}

#[uniffi::export]
impl GewuFilteredView {
    pub fn row_count(&self) -> u64 {
        self.inner.row_count()
    }

    pub fn unsorted(&self) -> Arc<GewuOrderedView> {
        Arc::new(GewuOrderedView {
            inner: self.inner.unsorted(),
        })
    }

    pub fn sort(&self, sorts: Vec<ColumnSort>) -> Result<Arc<GewuOrderedView>, GewuError> {
        let sorts: Vec<core::ColumnSort> = sorts.into_iter().map(Into::into).collect();
        Ok(Arc::new(GewuOrderedView {
            inner: self.inner.sort(&sorts)?,
        }))
    }

    pub fn count_nulls(&self, column: ColumnId) -> Result<u64, GewuError> {
        Ok(self.inner.count_nulls(column.into())?)
    }

    pub fn statistics(&self, column: ColumnId) -> Result<crate::NumericStatistics, GewuError> {
        Ok(self.inner.statistics(column.into())?.into())
    }

    pub fn distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<crate::Histogram, GewuError> {
        Ok(self
            .inner
            .distribution(column.into(), bins, scale.into())?
            .into())
    }

    pub fn values(&self, column: ColumnId) -> Result<crate::ValueSummary, GewuError> {
        Ok(self.inner.values(column.into())?.into())
    }

    pub fn lengths(&self, column: ColumnId, bins: u32) -> Result<crate::LengthSummary, GewuError> {
        Ok(self.inner.lengths(column.into(), bins)?.into())
    }

    pub fn scatter(
        &self,
        x_column: Option<ColumnId>,
        y_column: ColumnId,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<crate::ScatterResult, GewuError> {
        Ok(self
            .inner
            .scatter(
                x_column.map(Into::into),
                y_column.into(),
                limit,
                x_scale.into(),
                y_scale.into(),
            )?
            .into())
    }

    pub fn filter_choices(&self, column: ColumnId) -> Result<FilterChoices, GewuError> {
        Ok(self.inner.filter_choices(column.into())?.into())
    }

    pub fn filter_distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Option<RangeHistogram>, GewuError> {
        Ok(self
            .inner
            .filter_distribution(column.into(), bins, scale.into())?
            .map(Into::into))
    }
}
