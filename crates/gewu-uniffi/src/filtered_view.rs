//! The rows that pass a set of filters, with their analyses.
use std::sync::Arc;

use gewu_core::{
    self as core, AxisScale, ColumnFilter, ColumnId, ColumnSort, FilterChoices, Histogram,
    LengthSummary, NumericStatistics, RangeHistogram, ValueSummary,
};

use crate::{cache::Analyses, GewuError, GewuOrderedView, ScatterResult};

/// Rows of a dataset passing `filters`, in file order. Each analysis is
/// computed once per view and parameters, then shared by every caller and
/// every ordering of these rows.
#[derive(uniffi::Object)]
pub struct GewuFilteredView {
    pub(crate) dataset: Arc<core::Dataset>,
    pub(crate) inner: Arc<core::FilteredView>,
    pub(crate) filters: Vec<ColumnFilter>,
    analyses: Analyses,
}

impl GewuFilteredView {
    pub(crate) fn unfiltered(dataset: Arc<core::Dataset>) -> Arc<Self> {
        Arc::new(Self {
            inner: dataset.unfiltered(),
            dataset,
            filters: Vec::new(),
            analyses: Analyses::default(),
        })
    }

    pub(crate) fn new(
        dataset: Arc<core::Dataset>,
        filters: Vec<ColumnFilter>,
    ) -> Result<Arc<Self>, GewuError> {
        let inner = dataset.filter(&filters)?;
        Ok(Arc::new(Self {
            dataset,
            inner,
            filters,
            analyses: Analyses::default(),
        }))
    }

    /// These rows in file order.
    pub(crate) fn unsorted(self: &Arc<Self>) -> Arc<GewuOrderedView> {
        Arc::new(GewuOrderedView {
            inner: self.inner.unsorted(),
            filtered: Arc::clone(self),
            sorts: Vec::new(),
        })
    }

    /// These rows ordered by `sorts`.
    pub(crate) fn sorted(
        self: &Arc<Self>,
        sorts: Vec<ColumnSort>,
    ) -> Result<Arc<GewuOrderedView>, GewuError> {
        Ok(Arc::new(GewuOrderedView {
            inner: self.inner.sort(&sorts)?,
            filtered: Arc::clone(self),
            sorts,
        }))
    }
}

#[uniffi::export]
impl GewuFilteredView {
    pub fn row_count(&self) -> u64 {
        self.inner.row_count()
    }

    pub fn count_nulls(&self, column: ColumnId) -> Result<u64, GewuError> {
        Ok(self
            .analyses
            .null_counts
            .get_or_compute(column, (), || self.inner.count_nulls(column))?)
    }

    pub fn statistics(&self, column: ColumnId) -> Result<NumericStatistics, GewuError> {
        Ok(self
            .analyses
            .statistics
            .get_or_compute(column, (), || self.inner.statistics(column))?)
    }

    pub fn distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Histogram, GewuError> {
        Ok(self
            .analyses
            .distributions
            .get_or_compute(column, (bins, scale), || {
                self.inner.distribution(column, bins, scale)
            })?)
    }

    pub fn values(&self, column: ColumnId) -> Result<ValueSummary, GewuError> {
        Ok(self
            .analyses
            .values
            .get_or_compute(column, (), || self.inner.values(column))?)
    }

    pub fn lengths(&self, column: ColumnId, bins: u32) -> Result<LengthSummary, GewuError> {
        Ok(self
            .analyses
            .lengths
            .get_or_compute(column, bins, || self.inner.lengths(column, bins))?)
    }

    /// A sample of up to `limit` points. `x_column == None` plots against the
    /// source row number.
    pub fn scatter(
        &self,
        x_column: Option<ColumnId>,
        y_column: ColumnId,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<ScatterResult, GewuError> {
        let params = (x_column, y_column, limit, x_scale, y_scale);
        self.analyses.scatter.get_or_compute((), params, || {
            Ok(self
                .inner
                .scatter(x_column, y_column, limit, x_scale, y_scale)?
                .into())
        })
    }

    /// The values an editor for `column`'s filter can offer: those in rows
    /// passing every filter except `column`'s own, so its alternatives stay
    /// visible.
    pub fn filter_choices(&self, column: ColumnId) -> Result<FilterChoices, GewuError> {
        Ok(self
            .analyses
            .filter_choices
            .get_or_compute(column, (), || self.inner.filter_choices(column))?)
    }

    /// A range editor's histogram with exact bin endpoints, over the same rows
    /// as [`Self::filter_choices`]. `None` when the column has no plottable values.
    pub fn filter_distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Option<RangeHistogram>, GewuError> {
        Ok(self
            .analyses
            .filter_distributions
            .get_or_compute(column, (bins, scale), || {
                self.inner.filter_distribution(column, bins, scale)
            })?)
    }
}
