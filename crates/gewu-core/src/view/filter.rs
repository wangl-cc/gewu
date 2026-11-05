//! Filter editor candidates and distributions over visible rows.

use super::View;
use crate::{
    col_view::ColView,
    column::{predicate, value_text, Dictionary, FilterChoices, ValueChoice},
    stats, Cancellation, RangeHistogram, Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tally {
    counts: Vec<u64>,
    null_count: u64,
}

impl View {
    pub fn filter_choices(&self, column: u32, cancel: &Cancellation) -> Result<FilterChoices> {
        cancel.check()?;
        let view = self.col(column)?;
        if predicate::supports_equals(view.data_type()) {
            if let Some(dictionary) = view.column().dictionary(cancel)? {
                return dictionary_choices(view, dictionary, cancel);
            }
        }
        let null_count = stats::count_nulls(view, cancel)?;
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
            return Err(crate::AnalysisError::InvalidBinCount {
                operation: crate::AnalysisOperation::FilterDistribution,
                value: bins,
                min: 1,
                max: 60,
            }
            .into());
        }
        let view = self.col(column)?;
        let result = stats::build_scaled(view, bins, scale, cancel)?;
        cancel.check()?;
        Ok(result)
    }
}

fn dictionary_choices(
    view: ColView<'_>,
    dictionary: &Dictionary,
    cancel: &Cancellation,
) -> Result<FilterChoices> {
    cancel.check()?;
    let tally = tally_candidates(view, dictionary, cancel)?;
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
    cancel.check()?;
    Ok(FilterChoices {
        values: Some(values),
        null_count: tally.null_count,
    })
}

fn tally_candidates(
    view: ColView<'_>,
    dictionary: &Dictionary,
    cancel: &Cancellation,
) -> Result<Tally> {
    if view.is_complete() {
        return Ok(Tally {
            counts: dictionary.all_counts().to_vec(),
            null_count: dictionary.null_count(),
        });
    }
    let mut counts = [0u64; 256];
    let mut null_count = 0u64;
    for (index, &row) in view.rows().iter().enumerate() {
        cancel.check_at(index)?;
        let row = row as usize;
        if dictionary.is_null(row) {
            null_count += 1;
        } else {
            counts[dictionary.code_at(row) as usize] += 1;
        }
    }
    Ok(Tally {
        counts: counts[..dictionary.values().len()].to_vec(),
        null_count,
    })
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
        let dictionary = Dictionary::build(&array, &Cancellation::default())
            .unwrap()
            .unwrap();
        let code_b = dictionary.code_at(0);
        let code_a = dictionary.code_at(1);
        let selected = Fixture::new(&array, vec![0, 3]);
        let tally =
            tally_candidates(selected.view(), &dictionary, &Cancellation::default()).unwrap();
        assert_eq!(tally.null_count, 0);
        assert_eq!(tally.counts[code_a as usize], 0);
        assert_eq!(tally.counts[code_b as usize], 2);

        let all = Fixture::all(&array);
        let all = tally_candidates(all.view(), &dictionary, &Cancellation::default()).unwrap();
        assert_eq!(all.null_count, 1);
        assert_eq!(all.counts[code_a as usize], 1);
        assert_eq!(all.counts[code_b as usize], 2);
    }

    #[test]
    fn tally_handles_empty_and_all_null_columns() {
        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        let dictionary = Dictionary::build(&empty, &Cancellation::default())
            .unwrap()
            .unwrap();
        let all = Fixture::all(&empty);
        let tally = tally_candidates(all.view(), &dictionary, &Cancellation::default()).unwrap();
        assert_eq!(tally.null_count, 0);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
        let dictionary = Dictionary::build(&nulls, &Cancellation::default())
            .unwrap()
            .unwrap();
        let all = Fixture::all(&nulls);
        let tally = tally_candidates(all.view(), &dictionary, &Cancellation::default()).unwrap();
        assert_eq!(tally.null_count, 2);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(NullArray::new(3));
        let dictionary = Dictionary::build(&nulls, &Cancellation::default())
            .unwrap()
            .unwrap();
        let all = Fixture::all(&nulls);
        let tally = tally_candidates(all.view(), &dictionary, &Cancellation::default()).unwrap();
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
        let plain = Dictionary::build(&plain_array, &Cancellation::default())
            .unwrap()
            .unwrap();
        let native = Dictionary::build(&native_array, &Cancellation::default())
            .unwrap()
            .unwrap();
        let plain_view = Fixture::all(&plain_array);
        let native_view = Fixture::all(&native_array);
        let plain_tally =
            tally_candidates(plain_view.view(), &plain, &Cancellation::default()).unwrap();
        let native_tally =
            tally_candidates(native_view.view(), &native, &Cancellation::default()).unwrap();
        assert_eq!(plain_tally, native_tally);
    }
}
