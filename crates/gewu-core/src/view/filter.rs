//! Filter editor candidates and distributions over visible rows.

use std::sync::Arc;

use arrow::buffer::BooleanBuffer;

use super::FilteredView;
use crate::{
    col_view::ColView,
    column::{predicate, value_text, Dictionary, FilterChoices, ValueChoice},
    stats, ColumnId, RangeHistogram, Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tally {
    counts: Vec<u64>,
    null_count: u64,
}

impl FilteredView {
    /// The values an editor for `column`'s filter can offer: those in rows
    /// passing every filter except `column`'s own, so its alternatives stay
    /// visible. See [`Self::facet`].
    pub fn filter_choices(self: &Arc<Self>, column: ColumnId) -> Result<FilterChoices> {
        self.facet(column)?.choices(column)
    }

    /// A range editor's histogram for `column`, over the same rows as
    /// [`Self::filter_choices`].
    pub fn filter_distribution(
        self: &Arc<Self>,
        column: ColumnId,
        bins: u32,
        scale: crate::AxisScale,
    ) -> Result<Option<RangeHistogram>> {
        self.facet(column)?.range_histogram(column, bins, scale)
    }

    fn choices(&self, column: ColumnId) -> Result<FilterChoices> {
        let view = self.col(column)?;
        if predicate::supports_equals(view.data_type()) {
            if let Some(dictionary) = view.column().dictionary()? {
                return dictionary_choices(view, dictionary, self.selection_mask());
            }
        }
        Ok(FilterChoices {
            values: None,
            null_count: stats::count_nulls(view),
        })
    }

    fn range_histogram(
        &self,
        column: ColumnId,
        bins: u32,
        scale: crate::AxisScale,
    ) -> Result<Option<RangeHistogram>> {
        if !(1..=60).contains(&bins) {
            return Err(crate::AnalysisError::InvalidBinCount {
                operation: crate::AnalysisOperation::FilterDistribution,
                value: bins,
                min: 1,
                max: 60,
            }
            .into());
        }
        Ok(stats::build_scaled(self.col(column)?, bins, scale)?)
    }
}

fn dictionary_choices(
    view: ColView<'_>,
    dictionary: &Dictionary,
    selection: Option<&BooleanBuffer>,
) -> Result<FilterChoices> {
    let tally = match selection {
        Some(mask) if !view.is_complete() => tally_rows(mask.set_indices(), dictionary),
        _ => tally_candidates(view, dictionary),
    };
    let values = dictionary
        .labels()
        .iter()
        .enumerate()
        .filter(|(code, _)| tally.counts[*code] > 0)
        .map(|(code, label)| {
            Ok(ValueChoice {
                label: label.clone(),
                value: value_text::encode(dictionary.values(), code)?,
                count: tally.counts[code],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(FilterChoices {
        values: Some(values),
        null_count: tally.null_count,
    })
}

fn tally_candidates(view: ColView<'_>, dictionary: &Dictionary) -> Tally {
    if view.is_complete() {
        return Tally {
            counts: dictionary.all_counts().to_vec(),
            null_count: dictionary.null_count(),
        };
    }
    tally_rows(view.rows().iter(), dictionary)
}

/// Count the dictionary codes of the given source rows.
fn tally_rows(rows: impl Iterator<Item = usize>, dictionary: &Dictionary) -> Tally {
    let mut counts = [0u64; 256];
    let mut null_count = 0u64;
    for row in rows {
        if dictionary.is_null(row) {
            null_count += 1;
        } else {
            counts[dictionary.code_at(row) as usize] += 1;
        }
    }
    Tally {
        counts: counts[..dictionary.values().len()].to_vec(),
        null_count,
    }
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
