//! Filter inputs, editor data and facet queries.

use super::Session;
pub use crate::column::{DistinctResult, DistinctValue};
use crate::{
    predicate::Predicate, scope::RowScope, stats, types::Domain, Error, Filter, Histogram,
    QuerySpec, RangeHistogram, Result,
};

/// Complete bounded non-null candidates, or None when the column exceeds the
/// dictionary limit. Null counts are separate from selectable values.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterChoices {
    pub values: Option<Vec<ValueChoice>>,
    pub null_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueChoice {
    pub label: String,
    pub value: String,
    pub count: u64,
}

impl Session {
    /// Parse text once at the input boundary, retaining the column's full type.
    pub fn parse_bound(&self, column: u32, text: &str) -> Result<crate::Bound> {
        self.column(column)?.parse_bound(text)
    }

    pub fn filter_choices(&self, column: u32, spec: Option<&QuerySpec>) -> Result<FilterChoices> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        let nulls = col.array.logical_nulls();
        let null_count = rows
            .iter()
            .filter(|&row| nulls.as_ref().is_some_and(|nulls| nulls.is_null(row)))
            .count() as u64;
        let values = if col.capabilities().exact {
            col.dictionary()
                .map(|dictionary| {
                    dictionary
                        .distinct_values(&rows, true)
                        .into_iter()
                        .filter(|v| v.label.is_some())
                        .map(|v| {
                            Ok(ValueChoice {
                                label: v.label.unwrap(),
                                value: crate::column::value_text::encode(
                                    dictionary.values(),
                                    v.key as usize,
                                )?,
                                count: v.count,
                            })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
        } else {
            None
        };
        Ok(FilterChoices { values, null_count })
    }

    pub fn filter_distribution(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
        scale: crate::AxisScale,
    ) -> Result<Option<RangeHistogram>> {
        if !(1..=60).contains(&bins) {
            return Err(Error::InvalidArgument(
                "filter histogram bins must be between 1 and 60".into(),
            ));
        }
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        stats::build_scaled(stats::ArrayView::new(&col.array, &rows), bins, scale)
    }

    /// Capture a string column and facet rows for bounded candidate paging.
    /// Preparation evaluates the other filters and must run off the UI thread.
    /// The returned session has independent cancellation for its page scans;
    /// preparation itself completes before that cancellation handle is returned.
    pub fn prepare_text_choices(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
    ) -> Result<crate::TextChoices> {
        let col = self.column(column)?;
        let is_text = crate::types::domain(col.array.data_type()) == Some(Domain::Text);
        if !is_text {
            return Err(Error::InvalidArgument(
                "text choices require a string column".into(),
            ));
        }
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        crate::TextChoices::new(col.array.clone(), rows)
    }

    /// Distinct values of a column with their counts, for the checkbox filter.
    ///
    /// With `spec`, counts only rows that pass the filters on *other*
    /// columns (like Excel), and leaves out values with no such rows.
    ///
    /// A column with at most 255 non-null distinct values is encoded: the
    /// result is the first `limit` values in value order. A column with more is counted without an
    /// encoding: the result is the `limit` most common values (still in value
    /// order), `keyed` is false and the keys only number them.
    pub fn distinct_values(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        col.distinct_values(&rows, limit as usize)
    }

    /// Distribution of a numeric, date or time column in `bins` bins, for
    /// the range filter. With `spec`, counts only rows that pass the
    /// filters on *other* columns, like [`Self::distinct_values`].
    pub fn histogram(&self, column: u32, spec: Option<&QuerySpec>, bins: u32) -> Result<Histogram> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        stats::counts(stats::ArrayView::new(&col.array, &rows), bins)
    }

    /// Distribution with exact inclusive bounds for each bin. Counts use the
    /// same facet scope as [`Self::histogram`]; unbounded outer edges include
    /// infinities. Empty/all-NaN data returns no ranges. Unlike filter editor
    /// charts, this preserves the requested bin count.
    pub fn histogram_ranges(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
    ) -> Result<crate::RangeHistogram> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        stats::ranges(stats::ArrayView::new(&col.array, &rows), bins)
    }

    /// Validate and prepare an explicit filter without scanning data. Value-list
    /// filters may build the column's lazy index; other filters only parse scalars.
    pub fn validate_filter(&self, column: u32, filter: &Filter) -> Result<()> {
        self.prepare_filter(column, filter).map(|_| ())
    }

    pub(super) fn prepare_filter(&self, column_idx: u32, filter: &Filter) -> Result<Predicate<'_>> {
        self.column(column_idx)?.prepare_filter(filter)
    }
}
