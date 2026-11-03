//! Filter editor candidates and distributions over visible rows.

use super::View;
use crate::{column::FilterChoices, stats, Cancellation, Error, RangeHistogram, Result};

impl View {
    pub fn filter_choices(&self, column: u32, cancel: &Cancellation) -> Result<FilterChoices> {
        cancel.check()?;
        let col = self.column(column)?;
        let rows = self.selection();
        if col.capabilities().exact {
            if let Some(dictionary) = col.dictionary(cancel)? {
                return dictionary.filter_choices(&rows, cancel);
            }
        }
        let nulls = col.array().logical_nulls();
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
        let rows = self.selection();
        let result = stats::build_scaled(
            stats::ArrayView::new(col.array(), &rows),
            bins,
            scale,
            cancel,
        )?;
        cancel.check()?;
        Ok(result)
    }
}
