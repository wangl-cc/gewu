//! Typed column and scatter analyses over an immutable view.

use super::View;
use crate::{
    col_view::ColView, stats, AxisScale, Histogram, LengthSummary, NumericStatistics, Result,
    ValueSummary,
};

impl View {
    /// Bounded scatter sample over this immutable view.
    pub fn analyze_scatter(
        &self,
        x_column: Option<u32>,
        y_column: u32,
        limit: u32,
        x_scale: AxisScale,
        y_scale: AxisScale,
    ) -> Result<crate::ScatterResult> {
        let x = x_column
            .map(|column| self.column_array(column).map(|array| (column, array)))
            .transpose()?;
        let y = (y_column, self.column_array(y_column)?);
        Ok(stats::scatter::analyze(
            x, y, &self.rows, limit, x_scale, y_scale,
        )?)
    }

    /// Logical missing-value count in the requested view.
    pub fn count_nulls(&self, column: u32) -> Result<u64> {
        Ok(stats::count_nulls(self.col(column)?))
    }

    /// Native numeric statistics over the requested view. Execute off the UI thread.
    pub fn analyze_statistics(&self, column: u32) -> Result<NumericStatistics> {
        Ok(self.with_analysis_view(column, stats::analyze_statistics)?)
    }

    /// Numeric distribution; bins must be positive and the scale supported by the column.
    pub fn analyze_distribution(
        &self,
        column: u32,
        bins: u32,
        scale: AxisScale,
    ) -> Result<Histogram> {
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
    pub fn analyze_values(&self, column: u32) -> Result<ValueSummary> {
        Ok(self.with_analysis_view(column, stats::analyze_values)?)
    }

    /// Length statistics and a linear histogram; bins must be positive.
    pub fn analyze_lengths(&self, column: u32, bins: u32) -> Result<LengthSummary> {
        Ok(self.with_analysis_view(column, |view| {
            validate_bins(bins)?;
            stats::analyze_lengths(view, bins)
        })?)
    }

    fn with_analysis_view<T>(
        &self,
        column: u32,
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
