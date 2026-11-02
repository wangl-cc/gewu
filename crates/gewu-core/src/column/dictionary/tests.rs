use std::sync::Arc;

use arrow::{
    array::{
        DictionaryArray, Int16Array, Int32Array, Int64Array, Int8Array, NullArray, StringArray,
    },
    datatypes::{Int16Type, Int32Type, Int8Type},
};

use super::*;
use crate::{scope::Selection, Cancellation};

#[test]
fn codes_name_distinct_values_and_null_mask() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("dog"),
        None,
        Some("cat"),
        Some("dog"),
    ]));
    let dictionary = Dictionary::build(&a, &Cancellation::default())
        .unwrap()
        .unwrap();
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
    assert!(Dictionary::build(&at_limit, &Cancellation::default())
        .unwrap()
        .is_some());
    let over: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
    assert!(Dictionary::build(&over, &Cancellation::default())
        .unwrap()
        .is_none());
}

#[test]
fn nulls_do_not_consume_value_codes() {
    let array: ArrayRef = Arc::new(Int32Array::from_iter(
        (0..MAX_DISTINCT_VALUES as i32).map(Some).chain([None]),
    ));
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.n_values(), MAX_DISTINCT_VALUES);
    assert!(dictionary.has_null());
}

#[test]
fn all_counts_tally() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("b"),
        Some("a"),
        None,
        Some("b"),
    ]));
    let dictionary = Dictionary::build(&a, &Cancellation::default())
        .unwrap()
        .unwrap();
    let code_b = dictionary.code_at(0);
    let code_a = dictionary.code_at(1);
    let selection = Selection::Rows(Arc::new(vec![0, 3]));
    let tally = dictionary
        .tally(&selection, &Cancellation::default())
        .unwrap();
    assert_eq!(tally.null_count, 0);
    assert_eq!(tally.counts[code_a as usize], 0);
    assert_eq!(tally.counts[code_b as usize], 2);

    let all = dictionary
        .tally(&Selection::All(a.len()), &Cancellation::default())
        .unwrap();
    assert_eq!(all.null_count, 1);
    assert_eq!(all.counts[code_a as usize], 1);
    assert_eq!(all.counts[code_b as usize], 2);
}

#[test]
fn empty_and_all_null_columns_still_encode() {
    let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
    let dictionary = Dictionary::build(&empty, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.n_values(), 0);
    assert!(!dictionary.has_null());

    let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
    let dictionary = Dictionary::build(&nulls, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.n_values(), 0);
    assert!(dictionary.has_null());
    assert!(dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    let tally = dictionary
        .tally(&Selection::All(2), &Cancellation::default())
        .unwrap();
    assert_eq!(tally.null_count, 2);
    assert!(tally.counts.is_empty());
}

#[test]
fn predicates_match_null_and_values() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), None, Some("a")]));
    let dictionary = Arc::new(
        Dictionary::build(&a, &Cancellation::default())
            .unwrap()
            .unwrap(),
    );
    let mut hits = [false; 3];
    dictionary
        .matching_codes(&vec![false; dictionary.values().len()], false, true)
        .evaluate(0..3, &mut hits, &Cancellation::default())
        .unwrap();
    assert_eq!(hits, [false, true, false]);

    let mut hits = [false; 3];
    dictionary
        .matching_codes(&vec![false; dictionary.values().len()], true, false)
        .evaluate(0..3, &mut hits, &Cancellation::default())
        .unwrap();
    assert_eq!(hits, [true, false, true]);
}

#[test]
fn native_dictionary_reuses_keys_and_unifies_both_nulls() {
    let keys = Int8Array::from(vec![Some(0), None, Some(1), Some(2), Some(2), Some(0)]);
    let values = StringArray::from(vec![Some("b"), None, Some("a")]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(dictionary.has_null());

    let code_b = dictionary.code_at(0);
    let code_a = dictionary.code_at(3);
    assert_ne!(code_a, code_b);
    assert!(!dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    assert!(dictionary.is_null(2));
    assert!(!dictionary.is_null(3));

    let tally = dictionary
        .tally(&Selection::All(6), &Cancellation::default())
        .unwrap();
    assert_eq!(tally.null_count, 2);
    assert_eq!(tally.counts[code_a as usize], 2);
    assert_eq!(tally.counts[code_b as usize], 2);
}

#[test]
fn native_dictionary_drops_duplicate_and_unused_entries() {
    let keys = Int8Array::from(vec![Some(0), Some(1), Some(2)]);
    let values = StringArray::from(vec![Some("b"), Some("a"), Some("a"), Some("z")]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .unwrap();
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
    assert!(Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .is_none());
}

#[test]
fn native_dictionary_value_width_does_not_matter() {
    let keys = Int16Array::from(vec![Some(0), Some(1), Some(0)]);
    let values = Int16Array::from(vec![Some(1), Some(2)]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int16Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .unwrap();
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
    let dictionary = Arc::new(
        Dictionary::build(&a, &Cancellation::default())
            .unwrap()
            .unwrap(),
    );
    let code_b = dictionary.code_at(0);
    let mut selected = vec![false; dictionary.values().len()];
    selected[code_b as usize] = true;
    let predicate = dictionary.matching_codes(&selected, false, false);
    let cancel = Cancellation::default();

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
    let plain = Dictionary::build(&plain, &Cancellation::default())
        .unwrap()
        .unwrap();
    let native = Dictionary::build(&native, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(plain.n_values(), native.n_values());
    assert_eq!(plain.has_null(), native.has_null());
    let p_tally = plain
        .tally(&Selection::All(5), &Cancellation::default())
        .unwrap();
    let n_tally = native
        .tally(&Selection::All(5), &Cancellation::default())
        .unwrap();
    assert_eq!(p_tally, n_tally);
}

#[test]
fn arrow_null_type_encodes_every_row_as_null() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .expect("encoded");
    assert_eq!(dictionary.n_values(), 0);
    assert!(dictionary.has_null());
    assert!(dictionary.is_null(0));
    assert!(dictionary.is_null(1));
    assert!(dictionary.is_null(2));
    let tally = dictionary
        .tally(&Selection::All(3), &Cancellation::default())
        .unwrap();
    assert_eq!(tally.null_count, 3);
    assert!(tally.counts.is_empty());
}

#[test]
fn dictionary_labels_and_codes_are_natively_sorted() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![
        Some(100),
        Some(2),
        Some(10),
        Some(2),
    ]));
    let dictionary = Dictionary::build(&array, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.labels(), &["2", "10", "100"]);
    // code 0 -> 2, code 1 -> 10, code 2 -> 100
    assert_eq!(dictionary.code_at(0), 2); // 100 is rank 2
    assert_eq!(dictionary.code_at(1), 0); // 2 is rank 0
    assert_eq!(dictionary.code_at(2), 1); // 10 is rank 1
    assert_eq!(dictionary.code_at(3), 0); // 2 is rank 0
    let tally = dictionary
        .tally(&Selection::All(4), &Cancellation::default())
        .unwrap();
    assert_eq!(tally.counts, vec![2, 1, 1]);
}
