use std::sync::Arc;

use arrow::{
    array::{
        DictionaryArray, Int16Array, Int32Array, Int64Array, Int8Array, NullArray, StringArray,
    },
    datatypes::{Int16Type, Int32Type, Int8Type},
};

use super::*;
use crate::{engine::CancelToken, scope::Selection};

#[test]
fn codes_name_distinct_values_and_null_mask() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("dog"),
        None,
        Some("cat"),
        Some("dog"),
    ]));
    let dictionary = Dictionary::from_array(&a).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(dictionary.has_null());
    assert_eq!(dictionary.code_at(0), dictionary.code_at(3));
    assert_ne!(dictionary.code_at(0), dictionary.code_at(2));
    assert!(dictionary.is_null(1));
    assert!(!dictionary.is_null(0));
}

#[test]
fn too_many_values_are_not_encoded() {
    let at_limit: ArrayRef = Arc::new(Int32Array::from_iter_values(0..MAX_DISTINCT_VALUES as i32));
    assert!(Dictionary::from_array(&at_limit).is_some());
    let over: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
    assert!(Dictionary::from_array(&over).is_none());
}

#[test]
fn nulls_do_not_consume_value_codes() {
    let array: ArrayRef = Arc::new(Int32Array::from_iter(
        (0..MAX_DISTINCT_VALUES as i32).map(Some).chain([None]),
    ));
    let dictionary = Dictionary::from_array(&array).unwrap();
    assert_eq!(dictionary.n_values(), MAX_DISTINCT_VALUES);
    assert!(dictionary.has_null());
}

#[test]
fn all_counts_project_null_first_then_values() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("b"),
        Some("a"),
        None,
        Some("b"),
    ]));
    let dictionary = Dictionary::from_array(&a).unwrap();
    let code_b = dictionary.code_at(0);
    let code_a = dictionary.code_at(1);
    let selection = Selection::Rows(Arc::new(vec![0, 3]));
    let counts = dictionary.counts(&selection);
    assert_eq!(counts.null_count(), 0);
    assert_eq!(counts.count_at(code_a as usize), 0);
    assert_eq!(counts.count_at(code_b as usize), 2);

    // Projected iteration outputs null first, then each value in discovery order
    let projected: Vec<(Option<usize>, u64)> = counts.iter().collect();
    assert_eq!(projected, vec![
        (None, 0),
        (Some(code_b as usize), 2),
        (Some(code_a as usize), 0),
    ]);

    let all = dictionary.counts(&Selection::All(a.len()));
    assert_eq!(all.null_count(), 1);
    assert_eq!(all.count_at(code_a as usize), 1);
    assert_eq!(all.count_at(code_b as usize), 2);
}

#[test]
fn empty_and_all_null_columns_still_encode() {
    let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
    let dictionary = Dictionary::from_array(&empty).unwrap();
    assert_eq!(dictionary.n_values(), 0);
    assert!(!dictionary.has_null());

    let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
    let dictionary = Dictionary::from_array(&nulls).unwrap();
    assert_eq!(dictionary.n_values(), 0);
    assert!(dictionary.has_null());
    assert!(dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    let counts = dictionary.counts(&Selection::All(2));
    assert_eq!(counts.null_count(), 2);
    assert_eq!(counts.iter().collect::<Vec<_>>(), vec![(None, 2)]);
}

#[test]
fn predicates_match_null_and_values() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), None, Some("a")]));
    let dictionary = Arc::new(Dictionary::from_array(&a).unwrap());
    let mut hits = [false; 3];
    dictionary
        .prepare_predicate(&[dictionary.null_key()])
        .evaluate(0..3, &mut hits, &CancelToken::never())
        .unwrap();
    assert_eq!(hits, [false, true, false]);

    let mut hits = [false; 3];
    dictionary
        .prepare_predicate_with_exclude(&[dictionary.null_key()], true)
        .evaluate(0..3, &mut hits, &CancelToken::never())
        .unwrap();
    assert_eq!(hits, [true, false, true]);
}

#[test]
fn native_dictionary_reuses_keys_and_unifies_both_nulls() {
    let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(2), Some(2), Some(0)]);
    let values = StringArray::from(vec![Some("b"), None, Some("a")]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::from_array(&array).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(dictionary.has_null());

    let code_b = dictionary.code_at(0);
    let code_a = dictionary.code_at(3);
    assert_ne!(code_a, code_b);
    assert!(!dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    assert!(dictionary.is_null(2));
    assert!(!dictionary.is_null(3));

    let counts = dictionary.counts(&Selection::All(6));
    assert_eq!(counts.null_count(), 2);
    assert_eq!(counts.count_at(code_a as usize), 2);
    assert_eq!(counts.count_at(code_b as usize), 2);
}

#[test]
fn native_dictionary_drops_duplicate_and_unused_entries() {
    let keys = Int8Array::from(vec![Some(0), Some(1), Some(2)]);
    let values = StringArray::from(vec![Some("b"), Some("a"), Some("a"), Some("z")]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::from_array(&array).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(!dictionary.has_null());

    // Rows 1 and 2 both reference value "a" through different dictionary keys
    assert_eq!(dictionary.code_at(1), dictionary.code_at(2));
    assert_ne!(dictionary.code_at(0), dictionary.code_at(1));
}

#[test]
fn native_dictionary_over_the_limit_falls_back() {
    let values: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
    let keys = Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32);
    let array: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());
    assert!(Dictionary::from_array(&array).is_none());
}

#[test]
fn native_dictionary_value_width_does_not_matter() {
    let keys = Int16Array::from(vec![Some(0), Some(1), Some(0)]);
    let values = Int16Array::from(vec![Some(1), Some(2)]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int16Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::from_array(&array).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert_eq!(
        (0..3)
            .map(|row| dictionary.code_at(row))
            .collect::<Vec<_>>(),
        [0, 1, 0]
    );
}

#[test]
fn predicates_cover_ranges_indices_and_incoming_masks() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("b"),
        None,
        Some("a"),
        Some("b"),
    ]));
    let dictionary = Arc::new(Dictionary::from_array(&a).unwrap());
    let code_b = dictionary.code_at(0);
    let predicate = dictionary.prepare_predicate(&[code_b as u64]);
    let cancel = CancelToken::never();

    let mut range_hits = [false; 4];
    predicate.evaluate(0..4, &mut range_hits, &cancel).unwrap();
    assert_eq!(range_hits, [true, false, false, true]);

    // Rows selected out of order by index.
    let rows = [3usize, 1, 2, 0];
    let mut index_hits = [false; 4];
    predicate
        .evaluate(rows.as_slice(), &mut index_hits, &cancel)
        .unwrap();
    assert_eq!(index_hits, [true, false, false, true]);

    // Incoming mask intersects existing matches.
    let mut matched = [true; 4];
    let mut scratch = [false; 4];
    predicate
        .intersect(0..4, &mut matched, &mut scratch, &cancel)
        .unwrap();
    assert_eq!(matched, [true, false, false, true]);
}

#[test]
fn plain_and_native_dictionaries_agree() {
    let plain: ArrayRef = Arc::new(StringArray::from(vec![
        Some("b"),
        None,
        Some("a"),
        Some("b"),
        Some("a"),
    ]));
    let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(0), Some(1)]);
    let values = StringArray::from(vec![Some("b"), Some("a")]);
    let native: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
    let plain = Dictionary::from_array(&plain).unwrap();
    let native = Dictionary::from_array(&native).unwrap();
    assert_eq!(plain.n_values(), native.n_values());
    assert_eq!(plain.has_null(), native.has_null());
    let p_counts = plain.counts(&Selection::All(5));
    let n_counts = native.counts(&Selection::All(5));
    assert_eq!(p_counts.null_count(), n_counts.null_count());
    assert_eq!(p_counts.counts(), n_counts.counts());
}

#[test]
fn arrow_null_type_encodes_every_row_as_null() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let dictionary = Dictionary::from_array(&array).expect("encoded");
    assert_eq!(dictionary.n_values(), 0);
    assert!(dictionary.has_null());
    assert!(dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    assert!(dictionary.is_null(2));
    let counts = dictionary.counts(&Selection::All(3));
    assert_eq!(counts.null_count(), 3);
    assert_eq!(counts.iter().collect::<Vec<_>>(), vec![(None, 3)]);
}
