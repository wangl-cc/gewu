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
