//! Typed column and scatter analyses over an immutable view.

use super::View;
use crate::{
    stats::{self, ArrayView},
    AxisScale, Cancellation, ColumnOverview, Error, Histogram, LengthSummary, NumericStatistics,
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
        stats::scatter::analyze(x, y, &self.rows, limit, x_scale, y_scale, cancellation)
    }

    /// Logical row and missing-value counts in the requested view.
    pub fn analyze_overview(
        &self,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<ColumnOverview> {
        self.with_analysis_view(column, cancellation, |view| view.overview(cancellation))
    }

    /// Native numeric statistics over the requested view. Execute off the UI thread.
    pub fn analyze_statistics(
        &self,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<NumericStatistics> {
        self.with_analysis_view(column, cancellation, |view| {
            view.numeric_statistics(cancellation)
        })
    }

    /// Numeric distribution; bins must be positive and the scale supported by the column.
    pub fn analyze_distribution(
        &self,
        column: u32,
        bins: u32,
        scale: AxisScale,
        cancellation: &Cancellation,
    ) -> Result<Histogram> {
        self.with_analysis_view(column, cancellation, |view| {
            validate_bins(bins)?;
            if scale == AxisScale::Log10 && !stats::supports_logarithmic(view.data_type()) {
                return Err(Error::InvalidArgument(
                    "logarithmic binning requires a numeric distribution".into(),
                ));
            }
            view.histogram(bins, scale, cancellation)
        })
    }

    /// Estimated non-null cardinality and recounted candidate frequencies.
    pub fn analyze_values(&self, column: u32, cancellation: &Cancellation) -> Result<ValueSummary> {
        self.with_analysis_view(column, cancellation, |view| {
            view.value_summary(cancellation)
        })
    }

    /// Length statistics and a linear histogram; bins must be positive.
    pub fn analyze_lengths(
        &self,
        column: u32,
        bins: u32,
        cancellation: &Cancellation,
    ) -> Result<LengthSummary> {
        self.with_analysis_view(column, cancellation, |view| {
            validate_bins(bins)?;
            view.length_summary(bins, cancellation)
        })
    }

    fn with_analysis_view<T>(
        &self,
        column: u32,
        cancellation: &Cancellation,
        compute: impl FnOnce(ArrayView<'_>) -> Result<T>,
    ) -> Result<T> {
        cancellation.check()?;
        let col = self.column(column)?;
        let rows = self.selection();
        let result = compute(ArrayView::new(col.array(), &rows))?;
        cancellation.check()?;
        Ok(result)
    }
}

fn validate_bins(bins: u32) -> Result<()> {
    if bins == 0 {
        return Err(Error::InvalidArgument("bins must be at least 1".into()));
    }
    Ok(())
}
