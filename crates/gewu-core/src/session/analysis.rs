//! Analysis jobs and aggregate summaries over document views.

use arrow::array::Array;

use super::Session;
use crate::{
    scope::RowScope,
    stats::{self, AnalysisContext, ArrayView, AxisScale, ColumnSummary, Histogram},
    Result,
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
    ) -> Result<crate::ScatterAnalysis> {
        let x = x_column
            .map(|column| {
                self.column_array(column)
                    .map(|array| (column, array.clone()))
            })
            .transpose()?;
        let y = (y_column, self.column_array(y_column)?.clone());
        let view = self.views.snapshot(version)?;
        crate::ScatterAnalysis::new(x, y, view.rows.clone(), version, limit)
    }

    /// Distribution over the displayed rows, including this column's own
    /// filters and the text search. The axis still spans the source column.
    pub fn view_histogram(&self, version: u64, column: u32, bins: u32) -> Result<Histogram> {
        let col = self.column(column)?;
        let view = self.views.snapshot(version)?;
        let rows = self.rows_in(RowScope::Rows(view.rows.clone()))?;
        ArrayView::new(&col.array, &rows).histogram(bins, AxisScale::Linear, &AnalysisContext::default())
    }

    /// Available analysis sections, determined from type metadata without scanning rows.
    pub fn analysis_capabilities(&self, column: u32) -> Result<Vec<crate::AnalysisSection>> {
        Ok(stats::capabilities(
            self.column(column)?.array.data_type(),
        ))
    }

    /// Capture one column and the requested current view for an independently
    /// cancellable analysis. Rejects stale versions before creating the job.
    pub fn prepare_analysis(
        &self,
        version: u64,
        column: u32,
        section: crate::AnalysisSection,
    ) -> Result<crate::ColumnAnalysis> {
        let col = self.column(column)?;
        let view = self.views.snapshot(version)?;
        crate::ColumnAnalysis::new(
            col.array.clone(),
            view.rows.clone(),
            version,
            column,
            section,
        )
    }

    /// Summary of one column over the current view's rows, with up to
    /// `top` most common values for non-numeric columns.
    pub fn column_summary(&self, column: u32, top: u32) -> Result<ColumnSummary> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::Rows(self.views.current().rows.clone()))?;
        let view = ArrayView::new(&col.array, &rows);
        view.summary(col.choices(&rows, false), top as usize)
    }
}
