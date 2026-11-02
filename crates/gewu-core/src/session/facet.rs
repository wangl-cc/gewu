//! Filter inputs, editor data and facet queries.

use super::Session;
pub use crate::column::{FilterChoices, ValueChoice};
use crate::{
    scope::RowScope, stats, types::Domain, Cancellation, Error, Filter, QuerySpec, RangeHistogram,
    Result,
};

impl Session {
    /// Parse text once at the input boundary, retaining the column's full type.
    pub fn parse_bound(&self, column: u32, text: &str) -> Result<crate::Bound> {
        self.column(column)?.parse_bound(text)
    }

    pub fn filter_choices(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        cancel: &Cancellation,
    ) -> Result<FilterChoices> {
        cancel.check()?;
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column), cancel)?;
        if col.capabilities().exact {
            if let Some(dictionary) = col.dictionary(cancel)? {
                return dictionary.filter_choices(&rows, cancel);
            }
        }
        let nulls = col.array.logical_nulls();
        let mut null_count = 0;
        for (index, row) in rows.iter().enumerate() {
            cancel.check_at(index)?;
            null_count += u64::from(nulls.as_ref().is_some_and(|nulls| nulls.is_null(row)));
        }
        cancel.check()?;
        Ok(FilterChoices {
            values: None,
            null_count,
        })
    }

    pub fn filter_distribution(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
        scale: crate::AxisScale,
        cancel: &Cancellation,
    ) -> Result<Option<RangeHistogram>> {
        cancel.check()?;
        if !(1..=60).contains(&bins) {
            return Err(Error::InvalidArgument(
                "filter histogram bins must be between 1 and 60".into(),
            ));
        }
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column), cancel)?;
        let result = stats::build_scaled(
            stats::ArrayView::new(&col.array, &rows),
            bins,
            scale,
            cancel,
        )?;
        cancel.check()?;
        Ok(result)
    }

    /// The caller creates cancellation before queueing facet preparation.
    pub fn prepare_text_choices(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        cancel: &Cancellation,
    ) -> Result<crate::TextChoices> {
        cancel.check()?;
        let col = self.column(column)?;
        let is_text = crate::types::domain(col.array.data_type()) == Some(Domain::Text);
        if !is_text {
            return Err(Error::InvalidArgument(
                "text choices require a string column".into(),
            ));
        }
        let rows = self.rows_in(RowScope::facet(spec, column), cancel)?;
        cancel.check()?;
        crate::TextChoices::new(col.array.clone(), rows)
    }

    pub fn validate_filter(
        &self,
        column: u32,
        filter: &Filter,
        cancel: &Cancellation,
    ) -> Result<()> {
        self.column(column)?.prepare_filter(filter, cancel)?;
        cancel.check()
    }
}
