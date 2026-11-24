//! Filter editor candidates and distributions over visible rows.

use arrow::buffer::BooleanBuffer;

use super::{FilteredView, OrderedView};
use crate::{
    col_view::ColView,
    column::{predicate, value_text, Dictionary, FilterChoices, ValueChoice},
    stats, ColumnFilter, ColumnId, Filter, FilterError, FilterKind, RangeHistogram, Result,
};

impl OrderedView {
    /// Match the selected display cell by its exact value in the source column.
    /// Null selects nulls alone. An out-of-view display row returns `None`.
    pub fn exact_filter_for_cell(
        &self,
        column: ColumnId,
        display_row: u64,
    ) -> Result<Option<ColumnFilter>> {
        let array = self.column_array(column)?;
        let Some(source) = self.source_row_at(crate::ViewRow(display_row)) else {
            return Ok(None);
        };
        let row = source.0 as usize;
        let filter = if crate::column::is_null_at(array.as_ref(), row) {
            Filter::OnlyNulls
        } else if predicate::supports_equals(array.data_type()) {
            Filter::exact(vec![value_text::encode(array, row)?], false, false)
        } else {
            return Err(FilterError::UnsupportedType {
                condition: FilterKind::Exact,
                data_type: array.data_type().clone(),
            }
            .into());
        };
        Ok(Some(ColumnFilter { column, filter }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tally {
    counts: Vec<u64>,
    null_count: u64,
}

impl FilteredView {
    /// The values an editor for `column`'s filter can offer: those in rows
    /// passing every filter except `column`'s own, so its alternatives stay
    /// visible. See [`Self::facet`].
    pub fn filter_choices(&self, column: ColumnId) -> Result<FilterChoices> {
        let data = self.column(column)?;
        let mask = self.selection.facet_mask(column);
        if predicate::supports_equals(data.array().data_type()) {
            if let Some(dictionary) = data.dictionary()? {
                return dictionary_choices(dictionary, mask.as_ref());
            }
        }
        Ok(FilterChoices {
            values: None,
            null_count: stats::count_nulls(ColView::from_mask(data, mask.as_ref())),
        })
    }

    /// A range editor's histogram for `column`, over the same rows as
    /// [`Self::filter_choices`].
    pub fn filter_distribution(
        &self,
        column: ColumnId,
        bins: u32,
        scale: crate::AxisScale,
    ) -> Result<Option<RangeHistogram>> {
        let data = self.column(column)?;
        let bins =
            super::analysis::validate_bins(bins, 60, crate::AnalysisOperation::FilterDistribution)?;
        let mask = self.selection.facet_mask(column);
        Ok(stats::build_scaled(
            ColView::from_mask(data, mask.as_ref()),
            bins,
            scale,
        )?)
    }
}

fn dictionary_choices(
    dictionary: &Dictionary,
    mask: Option<&BooleanBuffer>,
) -> Result<FilterChoices> {
    let tally = tally_candidates(dictionary, mask);
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

fn tally_candidates(dictionary: &Dictionary, mask: Option<&BooleanBuffer>) -> Tally {
    if let Some(mask) = mask.filter(|mask| mask.count_set_bits() < mask.len()) {
        return tally_rows(mask.set_indices(), dictionary);
    }
    Tally {
        counts: dictionary.all_counts().to_vec(),
        null_count: dictionary.null_count(),
    }
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

    #[test]
    fn cell_filter_uses_display_order_and_lossless_temporal_values() {
        use arrow::array::{RecordBatch, TimestampNanosecondArray};

        use crate::{ColumnSort, Dataset};

        let times: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![
            Some(123_456_789),
            None,
            Some(123_456_788),
        ]));
        let order: ArrayRef = Arc::new(Int64Array::from(vec![2, 1, 3]));
        let batch = RecordBatch::try_from_iter([("time", times), ("order", order)]).unwrap();
        let dataset = Dataset::from_test_batch(batch);
        let time = dataset.column_id("time").unwrap();
        let sorted = dataset
            .unfiltered()
            .sort(&[ColumnSort {
                column: dataset.column_id("order").unwrap(),
                descending: true,
            }])
            .unwrap();
        for (display_row, source_row) in [(0, 2), (1, 0), (2, 1)] {
            let filter = sorted
                .exact_filter_for_cell(time, display_row)
                .unwrap()
                .unwrap();
            let selected = dataset.filter(&[filter]).unwrap().unsorted();
            assert_eq!(selected.source_rows(0, 10), [source_row]);
        }
        assert!(sorted.exact_filter_for_cell(time, 3).unwrap().is_none());
    }

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
        let mask = BooleanBuffer::from(vec![true, false, false, true]);
        let tally = tally_candidates(&dictionary, Some(&mask));
        assert_eq!(tally.null_count, 0);
        assert_eq!(tally.counts[code_a as usize], 0);
        assert_eq!(tally.counts[code_b as usize], 2);

        let all = tally_candidates(&dictionary, None);
        assert_eq!(all.null_count, 1);
        assert_eq!(all.counts[code_a as usize], 1);
        assert_eq!(all.counts[code_b as usize], 2);
    }

    #[test]
    fn tally_handles_empty_and_all_null_columns() {
        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        let dictionary = Dictionary::build(&empty).unwrap().unwrap();
        let tally = tally_candidates(&dictionary, None);
        assert_eq!(tally.null_count, 0);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
        let dictionary = Dictionary::build(&nulls).unwrap().unwrap();
        let tally = tally_candidates(&dictionary, None);
        assert_eq!(tally.null_count, 2);
        assert!(tally.counts.is_empty());

        let nulls: ArrayRef = Arc::new(NullArray::new(3));
        let dictionary = Dictionary::build(&nulls).unwrap().unwrap();
        let tally = tally_candidates(&dictionary, None);
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
        let plain_tally = tally_candidates(&plain, None);
        let native_tally = tally_candidates(&native, None);
        assert_eq!(plain_tally, native_tally);
    }
}
