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
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{ArrayRef, DictionaryArray, Int64Array, Int8Array, NullArray, StringArray},
        datatypes::Int8Type,
    };

    use super::*;
    use crate::col_view::test::Fixture;

    #[test]
    fn tally_uses_selected_rows_and_cached_full_counts() {
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
        ]));
        let dictionary = Dictionary::build(&array).unwrap().unwrap();
        let code_b = dictionary.code_at(0);
        let code_a = dictionary.code_at(1);
        let selected = Fixture::new(&array, vec![0, 3]);
        let tally = tally_candidates(selected.view(), &dictionary);
        assert_eq!(tally.null_count, 0);
        assert_eq!(tally.counts[code_a as usize], 0);
        assert_eq!(tally.counts[code_b as usize], 2);

        let all = Fixture::all(&array);
        let all = tally_candidates(all.view(), &dictionary);
        assert_eq!(all.null_count, 1);
        assert_eq!(all.counts[code_a as usize], 1);
        assert_eq!(all.counts[code_b as usize], 2);
    }

    #[test]
    fn tally_handles_empty_and_all_null_columns() {
        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        let dictionary = Dictionary::build(&empty).unwrap().unwrap();
        let all = Fixture::all(&empty);
        let tally = tally_candidates(all.view(), &dictionary);
        assert_eq!(tally.null_count, 0);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
        let dictionary = Dictionary::build(&nulls).unwrap().unwrap();
        let all = Fixture::all(&nulls);
        let tally = tally_candidates(all.view(), &dictionary);
        assert_eq!(tally.null_count, 2);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(NullArray::new(3));
        let dictionary = Dictionary::build(&nulls).unwrap().unwrap();
        let all = Fixture::all(&nulls);
        let tally = tally_candidates(all.view(), &dictionary);
        assert_eq!(tally.null_count, 3);
        assert!(tally.counts.is_empty());
    }

    #[test]
    fn native_and_plain_dictionary_tallies_match() {
        let plain_array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            None,
            Some("a"),
            Some("b"),
            Some("a"),
        ]));
        let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(0), Some(1)]);
        let values = StringArray::from(vec![Some("b"), Some("a")]);
        let native_array: ArrayRef =
            Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
        let plain = Dictionary::build(&plain_array).unwrap().unwrap();
        let native = Dictionary::build(&native_array).unwrap().unwrap();
        let plain_view = Fixture::all(&plain_array);
        let native_view = Fixture::all(&native_array);
        let plain_tally = tally_candidates(plain_view.view(), &plain);
        let native_tally = tally_candidates(native_view.view(), &native);
        assert_eq!(plain_tally, native_tally);
    }
}
