//! Typed column and scatter analyses over an immutable [`FilteredView`].

use std::num::NonZeroU32;

use super::FilteredView;
use crate::{
    col_view::ColView, stats, AxisScale, ColumnId, Histogram, LengthSummary, NumericStatistics,
    Result, ValueSummary,
};

impl FilteredView {
    /// Bounded scatter sample over this immutable view.
    pub fn scatter(
        &self,
        x_column: Option<ColumnId>,
        y_column: ColumnId,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<crate::ScatterResult> {
        let x = x_column
            .map(|column| self.column_array(column))
            .transpose()?;
        let y = self.column_array(y_column)?;
        Ok(stats::scatter::analyze(
            x,
            y,
            self.rows.as_slice(),
            limit,
            x_scale,
            y_scale,
        )?)
    }

    /// Logical missing-value count in the requested view.
    pub fn count_nulls(&self, column: ColumnId) -> Result<u64> {
        Ok(stats::count_nulls(self.col(column)?))
    }

    /// Native numeric statistics over the requested view. Execute off the UI thread.
    pub fn statistics(&self, column: ColumnId) -> Result<NumericStatistics> {
        Ok(self.with_analysis_view(column, stats::analyze_statistics)?)
    }

    /// Numeric distribution; bins must be positive and the scale supported by the column.
    pub fn distribution(&self, column: ColumnId, bins: u32, scale: AxisScale) -> Result<Histogram> {
        self.compute_distribution(column, bins, scale)
    }

    /// Distribution over the same filtered rows, with native inclusive bin boundaries
    /// for an inspector. The requested bin count is independent of the filter editor.
    pub fn distribution_with_ranges(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<crate::RangeHistogram> {
        self.compute_distribution(column, bins, scale)
    }

    fn compute_distribution<H: stats::HistogramOutput>(
        &self,
        column: ColumnId,
        bins: u32,
        scale: AxisScale,
    ) -> Result<H> {
        Ok(self.with_analysis_view(column, |view| {
            let bins = validate_bins(bins, u32::MAX, crate::AnalysisOperation::Distribution)?;
            stats::counts_scaled(view, bins, scale)
        })?)
    }

    /// Estimated non-null cardinality and recounted candidate frequencies.
    pub fn values(&self, column: ColumnId) -> Result<ValueSummary> {
        Ok(self.with_analysis_view(column, stats::analyze_values)?)
    }

    /// Length statistics and a linear histogram; bins must be positive.
    pub fn lengths(&self, column: ColumnId, bins: u32) -> Result<LengthSummary> {
        Ok(self.with_analysis_view(column, |view| {
            let bins = validate_bins(bins, u32::MAX, crate::AnalysisOperation::Distribution)?;
            stats::analyze_lengths(view, bins)
        })?)
    }

    fn with_analysis_view<T>(
        &self,
        column: ColumnId,
        compute: impl FnOnce(ColView<'_>) -> stats::Result<T>,
    ) -> stats::Result<T> {
        compute(self.col(column)?)
    }
}

pub(super) fn validate_bins(
    bins: u32,
    max: u32,
    operation: crate::AnalysisOperation,
) -> stats::Result<NonZeroU32> {
    NonZeroU32::new(bins)
        .filter(|n| n.get() <= max)
        .ok_or(crate::AnalysisError::InvalidBinCount {
            operation,
            value: bins,
            min: 1,
            max,
        })
}
