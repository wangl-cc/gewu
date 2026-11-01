use std::sync::Arc;

use arrow::{
    array::{
        BooleanArray, Date32Array, Decimal128Array, Decimal256Array, DictionaryArray,
        DurationSecondArray, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
        NullArray, StringArray, Time32MillisecondArray, TimestampMicrosecondArray,
    },
    compute::cast,
    datatypes::{i256, DataType, Int16Type, Int32Type, Int8Type, TimeUnit},
};

use super::{streaming::count_values, *};
use crate::{
    engine::CancelToken, format, predicate::Predicate, scope::Selection,
    session::facet::format_choices, DistinctValue, Result,
};

fn labels(dictionary: &Dictionary) -> Vec<Option<String>> {
    let formatter = format::formatter(dictionary.values.as_ref()).unwrap();
    (0..dictionary.values.len())
        .map(|code| format::cell(dictionary.values.as_ref(), &formatter, code))
        .collect()
}

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
fn float_counts_group_signed_zero_and_nan_payloads_separately_from_null() {
    let source = Float64Array::from(vec![
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(f64::from_bits(0x7ff8_1000_0000_0000)),
        Some(f64::from_bits(0xfff8_2000_0000_0000)),
        None,
        None,
        Some(1.0),
    ]);
    for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
        let array = arrow::compute::cast(&source, &data_type).unwrap();
        for rows in [
            Selection::All(array.len()),
            Selection::Rows(Arc::new(vec![7, 6, 5, 4, 3, 2, 1, 0])),
        ] {
            let result = count_values(&array, &rows, 8).unwrap();
            assert_eq!(result.distinct, 4);
            assert_eq!(result.top.len(), 4);
            for value in result.top {
                let expected = match value.label {
                    None => 2,
                    Some(label) => match label.parse::<f64>().unwrap() {
                        value if value.is_nan() => 3,
                        0.0 => 2,
                        1.0 => 1,
                        value => panic!("unexpected value {value}"),
                    },
                };
                assert_eq!(value.count, expected);
            }
        }
    }
}

#[test]
fn codes_follow_value_order_and_reserve_the_null_code() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("dog"),
        None,
        Some("cat"),
        Some("dog"),
    ]));
    let dictionary = Dictionary::build(&a).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(dictionary.has_null());
    assert_eq!(labels(&dictionary), vec![
        Some("cat".into()),
        Some("dog".into())
    ]);
    assert_eq!(
        (0..4)
            .map(|row| dictionary.code_at(row))
            .collect::<Vec<_>>(),
        [1, NULL_CODE, 0, 1]
    );
}

#[test]
fn numbers_sort_numerically() {
    let a: ArrayRef = Arc::new(Int32Array::from(vec![10, -3, 2, 10]));
    let dictionary = Dictionary::build(&a).unwrap();
    assert_eq!(labels(&dictionary), vec![
        Some("-3".into()),
        Some("2".into()),
        Some("10".into())
    ]);
    assert_eq!(dictionary.n_values(), 3);
}

#[test]
fn too_many_values_are_not_encoded() {
    let at_limit: ArrayRef = Arc::new(Int32Array::from_iter_values(0..MAX_DISTINCT_VALUES as i32));
    assert!(Dictionary::build(&at_limit).is_some());
    let over: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
    assert!(Dictionary::build(&over).is_none());
}

#[test]
fn nulls_do_not_consume_value_codes() {
    let array: ArrayRef = Arc::new(Int32Array::from_iter(
        (0..MAX_DISTINCT_VALUES as i32).map(Some).chain([None]),
    ));
    let dictionary = Dictionary::build(&array).unwrap();
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
    let dictionary = Dictionary::build(&a).unwrap();
    let selection = Selection::Rows(Arc::new(vec![0, 3]));
    let all = dictionary.counts_all(&selection).unwrap();
    assert_eq!(all, vec![
        DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 0
        },
        DistinctValue {
            key: 0,
            label: Some("a".into()),
            count: 0
        },
        DistinctValue {
            key: 1,
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
        0,
        1
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
    let dictionary = Dictionary::build(&a).unwrap();
    assert_eq!(
        dictionary
            .keys_for_labels(&[Some("a".into()), None])
            .unwrap(),
        vec![u64::from(NULL_CODE), 0]
    );
    assert!(dictionary
        .keys_for_labels(&[Some("missing".into())])
        .unwrap()
        .is_empty());
}

#[test]
fn empty_and_all_null_columns_still_encode() {
    let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
    let dictionary = Dictionary::build(&empty).unwrap();
    assert_eq!(dictionary.n_values(), 0);
    assert!(!dictionary.has_null());

    let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
    let dictionary = Dictionary::build(&nulls).unwrap();
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
    let dictionary = Arc::new(Dictionary::build(&a).unwrap());
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
    let dictionary = Dictionary::build(&array).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(dictionary.has_null());
    assert_eq!(labels(&dictionary), vec![
        Some("a".into()),
        Some("b".into())
    ]);
    assert_eq!(
        (0..6)
            .map(|row| dictionary.code_at(row))
            .collect::<Vec<_>>(),
        [1, NULL_CODE, NULL_CODE, 0, 0, 1]
    );
    assert_eq!(dictionary.counts(&Selection::All(6)).unwrap(), vec![
        DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: 2
        },
        DistinctValue {
            key: 0,
            label: Some("a".into()),
            count: 2
        },
        DistinctValue {
            key: 1,
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
    let dictionary = Dictionary::build(&array).unwrap();
    assert_eq!(dictionary.n_values(), 2);
    assert!(!dictionary.has_null());
    assert_eq!(labels(&dictionary), vec![
        Some("a".into()),
        Some("b".into())
    ]);
    assert_eq!(
        (0..3)
            .map(|row| dictionary.code_at(row))
            .collect::<Vec<_>>(),
        [1, 0, 0]
    );
}

#[test]
fn native_dictionary_over_the_limit_falls_back() {
    let values: ArrayRef = Arc::new(Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32));
    let keys = Int32Array::from_iter_values(0..=MAX_DISTINCT_VALUES as i32);
    let array: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap());
    assert!(Dictionary::build(&array).is_none());
}

#[test]
fn native_dictionary_value_width_does_not_matter() {
    let keys = Int16Array::from(vec![Some(0), Some(1), Some(0)]);
    let values = Int16Array::from(vec![Some(1), Some(2)]);
    let array: ArrayRef =
        Arc::new(DictionaryArray::<Int16Type>::try_new(keys, Arc::new(values)).unwrap());
    let dictionary = Dictionary::build(&array).unwrap();
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
    let dictionary = Arc::new(Dictionary::build(&a).unwrap());
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
    let plain = Dictionary::build(&plain).unwrap();
    let native = Dictionary::build(&native).unwrap();
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

#[test]
fn float_values_group_signed_zero_and_nan_payloads() {
    let source = Float64Array::from(vec![
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(f64::from_bits(0x7ff8_1000_0000_0000)),
        Some(f64::from_bits(0xfff8_2000_0000_0000)),
        None,
        Some(1.0),
    ]);
    for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
        let array = arrow::compute::cast(&source, &data_type).unwrap();
        let dictionary = Dictionary::build(&array).unwrap();
        assert_eq!(dictionary.n_values(), 3, "{data_type:?}");
        assert!(dictionary.has_null(), "{data_type:?}");

        let labels = labels(&dictionary);
        let parsed: Vec<Option<f64>> = labels
            .iter()
            .map(|label| label.as_deref().map(|text| text.parse::<f64>().unwrap()))
            .collect();
        assert_eq!(parsed[0], Some(0.0), "{data_type:?}");
        assert_eq!(parsed[1], Some(1.0), "{data_type:?}");
        assert!(parsed[2].unwrap().is_nan(), "{data_type:?}");

        let counts = dictionary.counts(&Selection::All(array.len())).unwrap();
        assert_eq!(counts.len(), 4, "{data_type:?}");
        assert_eq!(counts[0].label, None, "{data_type:?}");
        assert_eq!(counts[0].count, 1, "{data_type:?}");
        assert_eq!(counts[1].count, 2, "{data_type:?}");
        assert_eq!(counts[2].count, 1, "{data_type:?}");
        assert_eq!(counts[3].label.as_deref(), Some("NaN"), "{data_type:?}");
        assert_eq!(counts[3].count, 3, "{data_type:?}");

        // The streaming fallback must group the same way.
        let counted = count_values(&array, &Selection::All(array.len()), 10).unwrap();
        assert_eq!(counted.distinct as usize, counts.len(), "{data_type:?}");
    }
}

#[test]
fn counting_without_an_index() {
    let a: ArrayRef = Arc::new(StringArray::from(vec![
        Some("b"),
        Some("a"),
        None,
        Some("b"),
        Some("c"),
        Some("b"),
        Some("a"),
    ]));
    let all = count_values(&a, &Selection::All(a.len()), 2).unwrap();
    assert_eq!(all.distinct, 4);
    let top: Vec<_> = all
        .top
        .iter()
        .map(|v| (v.label.clone().unwrap(), v.count))
        .collect();
    assert_eq!(top, [("a".to_string(), 2), ("b".to_string(), 3)]);

    let some = count_values(&a, &Selection::Rows(Arc::new(vec![2, 4])), 10).unwrap();
    assert_eq!(some.distinct, 2);
    assert_eq!(some.top[0].label, None);
}

fn codes_of(dictionary: &Dictionary) -> Vec<u8> {
    (0..dictionary.codes.len())
        .map(|row| dictionary.code_at(row))
        .collect()
}

#[test]
fn integer_arrays_group_and_order_natively() {
    let signed: ArrayRef = Arc::new(Int64Array::from(vec![
        Some(-3),
        Some(7),
        Some(-3),
        None,
        Some(2),
    ]));
    for data_type in [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
    ] {
        let array = cast(&signed, &data_type).expect("integer cast");
        let dictionary = Dictionary::build(&array).expect("encoded");
        assert_eq!(
            labels(&dictionary),
            [Some("-3".into()), Some("2".into()), Some("7".into())],
            "{data_type}"
        );
        assert_eq!(codes_of(&dictionary), [0, 2, 0, 255, 1], "{data_type}");
        assert!(dictionary.has_null());
    }

    let unsigned: ArrayRef = Arc::new(arrow::array::UInt64Array::from(vec![
        Some(3),
        Some(7),
        Some(3),
        None,
        Some(2),
    ]));
    for data_type in [
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
    ] {
        let array = cast(&unsigned, &data_type).expect("unsigned cast");
        let dictionary = Dictionary::build(&array).expect("encoded");
        assert_eq!(
            labels(&dictionary),
            [Some("2".into()), Some("3".into()), Some("7".into())],
            "{data_type}"
        );
        assert_eq!(codes_of(&dictionary), [1, 2, 1, 255, 0], "{data_type}");
    }
}

#[test]
fn temporal_arrays_order_by_stored_unit() {
    let cases: [(ArrayRef, DataType); 4] = [
        (
            Arc::new(Date32Array::from(vec![
                Some(3),
                Some(1),
                Some(3),
                None,
                Some(2),
            ])),
            DataType::Date32,
        ),
        (
            Arc::new(Time32MillisecondArray::from(vec![
                Some(3_000),
                Some(1_000),
                Some(3_000),
                None,
                Some(2_000),
            ])),
            DataType::Time32(TimeUnit::Millisecond),
        ),
        (
            Arc::new(TimestampMicrosecondArray::from(vec![
                Some(3_000_000),
                Some(1_000_000),
                Some(3_000_000),
                None,
                Some(2_000_000),
            ])),
            DataType::Timestamp(TimeUnit::Microsecond, None),
        ),
        (
            Arc::new(DurationSecondArray::from(vec![
                Some(3),
                Some(1),
                Some(3),
                None,
                Some(2),
            ])),
            DataType::Duration(TimeUnit::Second),
        ),
    ];
    for (array, data_type) in cases {
        assert_eq!(array.data_type(), &data_type);
        let dictionary = Dictionary::build(&array).expect("encoded");
        assert_eq!(dictionary.n_values(), 3, "{data_type}");
        assert_eq!(codes_of(&dictionary), [2, 0, 2, 255, 1], "{data_type}");
    }
}

#[test]
fn decimal_arrays_order_by_coefficient() {
    let d128: ArrayRef = Arc::new(
        Decimal128Array::from(vec![Some(123_i128), Some(-45), Some(123), None, Some(0)])
            .with_precision_and_scale(10, 2)
            .unwrap(),
    );
    let dictionary = Dictionary::build(&d128).expect("encoded");
    assert_eq!(labels(&dictionary), [
        Some("-0.45".into()),
        Some("0.00".into()),
        Some("1.23".into())
    ]);
    assert_eq!(codes_of(&dictionary), [2, 0, 2, 255, 1]);

    let d256: ArrayRef = Arc::new(
        Decimal256Array::from(vec![
            Some(i256::from_i128(5)),
            Some(i256::from_i128(-2)),
            None,
            Some(i256::from_i128(5)),
        ])
        .with_precision_and_scale(76, 0)
        .unwrap(),
    );
    let dictionary = Dictionary::build(&d256).expect("encoded");
    assert_eq!(dictionary.n_values(), 2);
    assert_eq!(codes_of(&dictionary), [1, 0, 255, 1]);
}

#[test]
fn boolean_arrays_order_false_before_true() {
    let array: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        Some(false),
        Some(true),
        None,
    ]));
    let dictionary = Dictionary::build(&array).expect("encoded");
    assert_eq!(labels(&dictionary), [
        Some("false".into()),
        Some("true".into())
    ]);
    assert_eq!(codes_of(&dictionary), [1, 0, 1, 255]);
}

#[test]
fn arrow_null_type_encodes_every_row_as_null() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let dictionary = Dictionary::build(&array).expect("encoded");
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
