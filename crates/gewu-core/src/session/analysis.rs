//! Analysis jobs and aggregate summaries over document views.

use arrow::{
    array::{make_comparator, Array, ArrayRef},
    compute::SortOptions,
};

use super::Session;
use crate::{
    analysis::numeric::Numeric,
    column::stats,
    format,
    scope::{for_rows, RowScope, Selection},
    session::facet::format_choices,
    ColumnSummary, Histogram, Result,
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
        stats::counts(&col.array, &rows, bins)
    }

    /// Available analysis sections, determined from type metadata without scanning rows.
    pub fn analysis_capabilities(&self, column: u32) -> Result<Vec<crate::AnalysisSection>> {
        Ok(crate::analysis::capabilities(
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
        let array = &col.array;
        let rows = self.rows_in(RowScope::Rows(self.views.current().rows.clone()))?;
        let mut s = ColumnSummary {
            rows: rows.len() as u64,
            null_count: 0,
            nan_count: 0,
            distinct_count: 0,
            min: None,
            max: None,
            min_label: None,
            max_label: None,
            mean: None,
            std_dev: None,
            top_values: Vec::new(),
        };
        let f = format::formatter(array.as_ref())?;
        let label = |row: Option<usize>| row.and_then(|r| format::cell(array.as_ref(), &f, r));
        if let Some(numeric) = Numeric::new(array) {
            let context = crate::analysis::AnalysisContext::default();
            let details = for_rows!(&rows, |it| numeric.summarize(it, &mut s, true, &context))?;
            (s.min_label, s.max_label) = (label(details.min_row), label(details.max_row));
            return Ok(s);
        }

        let top = top as usize;
        s.null_count = match &rows {
            Selection::All(_) => array.null_count() as u64,
            Selection::Rows(_) => rows.iter().filter(|&r| array.is_null(r)).count() as u64,
        };
        let dictionary = self.dictionary(column)?;
        if let Some(dictionary) = dictionary {
            let counts = dictionary.tally(&rows);
            let mut choices = format_choices(
                dictionary.values().as_ref(),
                dictionary.has_null(),
                &counts,
                false,
            )?;
            s.distinct_count = choices.len() as u64;
            let mut present = choices.iter().filter_map(|v| v.label.as_ref());
            s.min_label = present.clone().next().cloned();
            s.max_label = present.next_back().cloned();
            if s.null_count > 0 {
                s.distinct_count -= 1;
            }
            choices.retain(|v| v.label.is_some());
            choices.sort_by_key(|v| std::cmp::Reverse(v.count));
            choices.truncate(top);
            s.top_values = choices;
        } else {
            // One spare in case null is among the most common.
            let counted =
                crate::column::dictionary::streaming::count_values(array, &rows, top + 1)?;
            s.distinct_count = counted.distinct;
            let (lo, hi) = extremes(array, &rows)?;
            (s.min_label, s.max_label) = (label(lo), label(hi));
            if s.null_count > 0 {
                s.distinct_count -= 1;
            }
            let mut values = counted.top;
            values.retain(|v| v.label.is_some());
            values.sort_by_key(|v| std::cmp::Reverse(v.count));
            values.truncate(top);
            s.top_values = values;
        }
        Ok(s)
    }
}

/// Rows holding the smallest and largest non-null values among `rows`, in
/// the column's own order. `None` for types Arrow cannot compare. (Numeric
/// columns get theirs from the summary, which also leaves out NaN.)
fn extremes(array: &ArrayRef, rows: &Selection) -> Result<(Option<usize>, Option<usize>)> {
    let Ok(cmp) = make_comparator(array.as_ref(), array.as_ref(), SortOptions::default()) else {
        return Ok((None, None));
    };
    let (mut lo, mut hi) = (None::<usize>, None::<usize>);
    for r in rows.iter().filter(|&r| array.is_valid(r)) {
        if lo.is_none_or(|l| cmp(r, l).is_lt()) {
            lo = Some(r);
        }
        if hi.is_none_or(|h| cmp(r, h).is_gt()) {
            hi = Some(r);
        }
    }
    Ok((lo, hi))
}
