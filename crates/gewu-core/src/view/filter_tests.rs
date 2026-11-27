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
