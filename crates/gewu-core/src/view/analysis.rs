//! Typed column and scatter analyses over an immutable [`FilteredView`].

use super::FilteredView;
use crate::{
    AxisScale, ColumnId, Histogram, LengthSummary, NumericStatistics, Result, ValueSummary,
    col_view::ColView, stats,
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
            .map(|column| self.column_array(column).map(|array| (column, array)))
            .transpose()?;
        let y = (y_column, self.column_array(y_column)?);
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
            validate_bins(bins)?;
            if scale == AxisScale::Log10 && !stats::supports_logarithmic(view.data_type()) {
                return Err(crate::AnalysisError::UnsupportedOperation {
                    operation: crate::AnalysisOperation::Distribution,
                    data_type: view.data_type().clone(),
                });
            }
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
            validate_bins(bins)?;
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

fn validate_bins(bins: u32) -> stats::Result<()> {
    if bins == 0 {
        return Err(crate::AnalysisError::InvalidBinCount {
            operation: crate::AnalysisOperation::Distribution,
            value: bins,
            min: 1,
            max: u32::MAX,
        });
    }
    Ok(())
}
