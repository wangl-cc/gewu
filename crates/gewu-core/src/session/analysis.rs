//! Typed column analyses over view snapshots and separately prepared scatter jobs.

use arrow::array::Array;

use super::Session;
use crate::{
    scope::Selection,
    stats::{self, ArrayView},
    AnalysisField, AnalysisSection, AxisScale, Cancellation, ColumnOverview, Error, Histogram,
    LengthSummary, NumericStatistics, Result, ScatterAnalysis, ValueSummary,
};

impl Session {
    /// Capture an independently cancellable scatter sample. X=None uses source
    /// row IDs; explicit axes must support the column's scatter capability.
    /// Rejects stale views and limits outside 1..=MAX_SCATTER_POINTS.
    pub fn prepare_scatter(
        &self,
        version: u64,
        x_column: Option<u32>,
        y_column: u32,
        limit: u32,
    ) -> Result<ScatterAnalysis> {
        let x = x_column
            .map(|column| {
                self.column_array(column)
                    .map(|array| (column, array.clone()))
            })
            .transpose()?;
        let y = (y_column, self.column_array(y_column)?.clone());
        let view = self.views.snapshot(version)?;
        ScatterAnalysis::new(x, y, view.rows.clone(), version, limit)
    }

    /// Available analysis sections, determined from type metadata without scanning rows.
    pub fn analysis_capabilities(&self, column: u32) -> Result<Vec<AnalysisSection>> {
        Ok(stats::capabilities(self.column(column)?.array.data_type()))
    }

    /// Logical row and missing-value counts in the requested view.
    pub fn analyze_overview(
        &self,
        version: u64,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<ColumnOverview> {
        self.with_analysis_view(
            version,
            column,
            AnalysisSection::Overview,
            cancellation,
            |view| view.overview(cancellation),
        )
    }

    /// Native numeric statistics over the requested view. Execute off the UI thread.
    pub fn analyze_statistics(
        &self,
        version: u64,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<NumericStatistics> {
        self.with_analysis_view(
            version,
            column,
            AnalysisSection::Statistics,
            cancellation,
            |view| view.numeric_statistics(cancellation),
        )
    }

    /// Numeric distribution; bins must be positive and the scale supported by the column.
    pub fn analyze_distribution(
        &self,
        version: u64,
        column: u32,
        bins: u32,
        scale: AxisScale,
        cancellation: &Cancellation,
    ) -> Result<Histogram> {
        self.with_analysis_view(
            version,
            column,
            AnalysisSection::Distribution,
            cancellation,
            |view| {
                validate_bins(bins)?;
                if scale == AxisScale::Log10 && !stats::supports_logarithmic(view.data_type()) {
                    return Err(Error::InvalidArgument(
                        "logarithmic binning requires a numeric distribution".into(),
                    ));
                }
                view.histogram(bins, scale, cancellation)
            },
        )
    }

    /// Estimated non-null cardinality and recounted candidate frequencies.
    pub fn analyze_values(
        &self,
        version: u64,
        column: u32,
        cancellation: &Cancellation,
    ) -> Result<ValueSummary> {
        self.with_analysis_view(
            version,
            column,
            AnalysisSection::Values,
            cancellation,
            |view| view.value_summary(cancellation),
        )
    }

    /// Length statistics and a linear histogram; bins must be positive.
    pub fn analyze_lengths(
        &self,
        version: u64,
        column: u32,
        bins: u32,
        cancellation: &Cancellation,
    ) -> Result<LengthSummary> {
        self.with_analysis_view(
            version,
            column,
            AnalysisSection::Lengths,
            cancellation,
            |view| {
                validate_bins(bins)?;
                view.length_summary(bins, cancellation)
            },
        )
    }

    /// Nested field metadata, independent of the active view. Does not scan rows.
    pub fn column_structure(&self, column: u32) -> Result<Vec<AnalysisField>> {
        stats::analysis::structure(self.column(column)?.array.data_type())
    }

    // Acquire the snapshot when execution starts. Later view commits do not invalidate
    // these shared buffers; only the caller's cancellation signal stops this request.
    fn with_analysis_view<T>(
        &self,
        version: u64,
        column: u32,
        section: AnalysisSection,
        cancellation: &Cancellation,
        compute: impl FnOnce(ArrayView<'_>) -> Result<T>,
    ) -> Result<T> {
        cancellation.check()?;
        let col = self.column(column)?;
        let snapshot = self.views.snapshot(version)?;
        stats::analysis::ensure_supported(col.array.data_type(), section)?;
        // These analyses are order-independent, including over a fully sorted view.
        let rows = if snapshot.rows.len() == col.array.len() {
            Selection::All(col.array.len())
        } else {
            Selection::Rows(snapshot.rows.clone())
        };
        let result = compute(ArrayView::new(&col.array, &rows))?;
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

#[cfg(all(test, feature = "csv"))]
mod tests {
    use super::*;
    use crate::{Filter, QuerySpec};

    #[test]
    fn running_analysis_keeps_snapshot_and_observes_final_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("analysis.csv");
        std::fs::write(&path, "number\n1\n2\n3\n").unwrap();
        let session = Session::open(path).unwrap();
        let cancellation = Cancellation::default();
        let statistics = session
            .with_analysis_view(0, 0, AnalysisSection::Statistics, &cancellation, |view| {
                session
                    .prepare_view(&QuerySpec::single(
                        0,
                        Filter::range(Some("2".into()), None, false),
                    ))?
                    .run()?;
                // The commit occurred after snapshot acquisition and before the scan.
                view.numeric_statistics(&cancellation)
            })
            .unwrap();
        assert_eq!(statistics.mean, Some(2.0));
        assert_eq!(
            session
                .analyze_statistics(session.current_view().version, 0, &cancellation)
                .unwrap()
                .mean,
            Some(2.5)
        );

        let result = session.with_analysis_view(
            session.current_view().version,
            0,
            AnalysisSection::Overview,
            &cancellation,
            |view| {
                let result = view.overview(&cancellation)?;
                cancellation.cancel();
                Ok(result)
            },
        );
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(session
            .analyze_overview(session.current_view().version, 0, &Cancellation::default())
            .is_ok());
    }
}
