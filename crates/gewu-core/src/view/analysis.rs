//! Typed column and scatter analyses over an immutable view.

use super::View;
use crate::{
    col_view::ColView, stats, AxisScale, Cancellation, Histogram, LengthSummary, NumericStatistics,
    Result, ValueSummary,
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
        cancellation: &Cancellation,
    ) -> Result<crate::ScatterResult> {
        cancellation.check()?;
        let x = x_column
            .map(|column| self.column_array(column).map(|array| (column, array)))
            .transpose()?;
        let y = (y_column, self.column_array(y_column)?);
        Ok(stats::scatter::analyze(
            x,
            y,
            &self.rows,
            limit,
            x_scale,
            y_scale,
            cancellation,
        )?)
    }

    /// Logical missing-value count in the requested view.
    pub fn count_nulls(&self, column: u32, cancellation: &Cancellation) -> Result<u64> {
        cancellation.check()?;
        let result = stats::count_nulls(self.col(column)?, cancellation)?;
        cancellation.check()?;
        Ok(result)
    }

    /// Native numeric statistics over the requested view. Execute off the UI thread.
    pub fn analyze_statistics(
        &self,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<NumericStatistics> {
        Ok(self.with_analysis_view(column, cancellation, |view| {
            stats::analyze_statistics(view, cancellation)
        })?)
    }

    /// Numeric distribution; bins must be positive and the scale supported by the column.
    pub fn analyze_distribution(
        &self,
        column: u32,
        bins: u32,
        scale: AxisScale,
        cancellation: &Cancellation,
    ) -> Result<Histogram> {
        Ok(self.with_analysis_view(column, cancellation, |view| {
            validate_bins(bins)?;
            if scale == AxisScale::Log10 && !stats::supports_logarithmic(view.data_type()) {
                return Err(crate::AnalysisError::UnsupportedOperation {
                    operation: crate::AnalysisOperation::Distribution,
                    data_type: view.data_type().clone(),
                });
            }
            stats::counts_scaled(view, bins, scale, cancellation)
        })?)
    }

    /// Estimated non-null cardinality and recounted candidate frequencies.
    pub fn analyze_values(&self, column: u32, cancellation: &Cancellation) -> Result<ValueSummary> {
        Ok(self.with_analysis_view(column, cancellation, |view| {
            stats::analyze_values(view, cancellation)
        })?)
    }

    /// Length statistics and a linear histogram; bins must be positive.
    pub fn analyze_lengths(
        &self,
        column: u32,
        bins: u32,
        cancellation: &Cancellation,
    ) -> Result<LengthSummary> {
        Ok(self.with_analysis_view(column, cancellation, |view| {
            validate_bins(bins)?;
            stats::analyze_lengths(view, bins, cancellation)
        })?)
    }

    fn with_analysis_view<T>(
        &self,
        column: u32,
        cancellation: &Cancellation,
        compute: impl FnOnce(ColView<'_>) -> stats::Result<T>,
    ) -> stats::Result<T> {
        cancellation.check()?;
        let result = compute(self.col(column)?)?;
        cancellation.check()?;
        Ok(result)
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
