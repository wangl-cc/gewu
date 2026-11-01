use std::sync::Arc;

use arrow::{
    array::{
        DictionaryArray, Int16Array, Int32Array, Int64Array, Int8Array, NullArray, StringArray,
    },
    datatypes::{Int16Type, Int32Type, Int8Type},
};

use super::*;
use crate::{
    engine::CancelToken, format, predicate::Predicate, scope::Selection,
    session::facet::format_choices, DistinctValue, Result,
};

impl Dictionary {
    fn counts(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        format_choices(
            self.values.as_ref(),
            self.has_null,
            &self.tally(rows),
            false,
        )
    }

    fn counts_all(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        format_choices(self.values.as_ref(), self.has_null, &self.tally(rows), true)
    }

    fn keys_for_labels(&self, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let target: std::collections::HashSet<&Option<String>> = labels.iter().collect();
        let mut keys = Vec::new();
        if self.has_null && labels.iter().any(Option::is_none) {
            keys.push(u64::from(NULL_CODE));
        }
        let formatter = format::formatter(self.values.as_ref())?;
        for code in 0..self.values.len() {
            let label = format::cell(self.values.as_ref(), &formatter, code);
            if target.contains(&label) {
                keys.push(code as u64);
            }
        }
        Ok(keys)
    }

    fn prepare_text_predicate(
        self: &Arc<Self>,
        values: &[Option<String>],
        exclude: bool,
    ) -> Result<Predicate<'static>> {
        let keys = self.keys_for_labels(values)?;
        let mut allowed = [exclude; 256];
        for key in keys {
            if let Some(slot) = allowed.get_mut(key as usize) {
                *slot = !exclude;
            }
        }
        Ok(self.predicate_from_allowed(allowed))
    }
}

#[test]
fn codes_name_distinct_values_and_reserve_the_null_code() {
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
    assert_eq!(dictionary.code_at(1), NULL_CODE);
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
fn all_counts_keep_zero_count_options_and_stable_keys() {
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
    let all = dictionary.counts_all(&selection).unwrap();
    assert_eq!(all, vec![
        DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 0
        },
        DistinctValue {
            key: u64::from(code_a),
            label: Some("a".into()),
            count: 0
        },
        DistinctValue {
            key: u64::from(code_b),
            label: Some("b".into()),
            count: 2
        },
    ]);
    assert_eq!(dictionary.counts(&selection).unwrap(), vec![all[2].clone()]);
    let empty = dictionary
        .counts_all(&Selection::Rows(Arc::new(vec![])))
        .unwrap();
    assert_eq!(empty.iter().map(|value| value.key).collect::<Vec<_>>(), [
        u64::from(NULL_CODE),
        u64::from(code_a),
        u64::from(code_b),
    ]);
    assert!(empty.iter().all(|value| value.count == 0));
    assert_eq!(
        dictionary.counts_all(&Selection::All(a.len())).unwrap(),
        dictionary.counts(&Selection::All(a.len())).unwrap()
    );
}

#[test]
fn label_lookup_maps_null_to_the_reserved_key() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), Some("a"), None]));
    let dictionary = Dictionary::from_array(&a).unwrap();
    let code_a = dictionary.code_at(1);
    assert_eq!(
        dictionary
            .keys_for_labels(&[Some("a".into()), None])
            .unwrap(),
        vec![u64::from(NULL_CODE), u64::from(code_a)]
    );
    assert!(dictionary
        .keys_for_labels(&[Some("missing".into())])
        .unwrap()
        .is_empty());
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
    assert_eq!(dictionary.code_at(0), NULL_CODE);
    assert_eq!(dictionary.counts(&Selection::All(2)).unwrap(), vec![
        DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 2
        }
    ]);
}

#[test]
fn predicates_match_null_through_the_reserved_code() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![Some("b"), None, Some("a")]));
    let dictionary = Arc::new(Dictionary::from_array(&a).unwrap());
    let mut hits = [false; 3];
    dictionary
        .prepare_predicate(&[u64::from(NULL_CODE)])
        .evaluate(0..3, &mut hits, &CancelToken::never())
        .unwrap();
    assert_eq!(hits, [false, true, false]);

    let mut hits = [false; 3];
    dictionary
        .prepare_text_predicate(&[None], false)
        .unwrap()
        .evaluate(0..3, &mut hits, &CancelToken::never())
        .unwrap();
    assert_eq!(hits, [false, true, false]);
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
    assert_eq!(
        (0..6)
            .map(|row| dictionary.code_at(row))
            .collect::<Vec<_>>(),
        [code_b, NULL_CODE, NULL_CODE, code_a, code_a, code_b]
    );
    assert_eq!(dictionary.counts(&Selection::All(6)).unwrap(), vec![
        DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 2
        },
        DistinctValue {
            key: u64::from(code_a),
            label: Some("a".into()),
            count: 2
        },
        DistinctValue {
            key: u64::from(code_b),
            label: Some("b".into()),
            count: 2
        },
    ]);
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
    let keys = dictionary.keys_for_labels(&[Some("b".into())]).unwrap();
    let predicate = dictionary.prepare_predicate(&keys);
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
    assert_eq!(
        plain.counts_all(&Selection::All(5)).unwrap(),
        native.counts_all(&Selection::All(5)).unwrap()
    );
    for labels in [
        vec![Some("a".into())],
        vec![Some("b".into())],
        vec![None],
        vec![Some("a".into()), None],
    ] {
        assert_eq!(
            plain.keys_for_labels(&labels).unwrap(),
            native.keys_for_labels(&labels).unwrap()
        );
    }
}

fn codes_of(dictionary: &Dictionary) -> Vec<u8> {
    (0..dictionary.codes.len())
        .map(|row| dictionary.code_at(row))
        .collect()
}

#[test]
fn arrow_null_type_encodes_every_row_as_null() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let dictionary = Dictionary::from_array(&array).expect("encoded");
    assert_eq!(dictionary.n_values(), 0);
    assert!(dictionary.has_null());
    assert_eq!(codes_of(&dictionary), [255, 255, 255]);
    let present = dictionary.counts(&Selection::All(3)).unwrap();
    assert_eq!(present, [DistinctValue {
        key: u64::from(NULL_CODE),
        label: None,
        count: 3
    }]);
}
