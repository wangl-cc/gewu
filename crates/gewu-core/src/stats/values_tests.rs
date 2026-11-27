use std::sync::Arc;

use arrow::array::{ArrayRef, DictionaryArray, Float64Array, Int32Array, Int64Array, StringArray};

use super::*;

fn hash(value: usize) -> u64 {
    hash_native(value as u64)
}

#[test]
fn dictionary_construction_errors_reach_value_analysis() {
    let array: ArrayRef = Arc::new(
        arrow::array::TimestampSecondArray::from(vec![0]).with_timezone("Unknown/Timezone"),
    );
    let fixture = crate::col_view::test::Fixture::all(&array);
    assert!(matches!(
        analyze(fixture.view()),
        Err(crate::AnalysisError::Column(crate::ColumnError::Internal(
            crate::InternalError::Arrow(_)
        )))
    ));
}

#[test]
fn bounded_sketch_preserves_heavy_hitters_and_recounts_candidates() {
    let mut sketch = Sketch::default();
    for row in 0..50_000 {
        sketch.add(hash(0), row);
    }
    for row in 50_000..1_050_000 {
        sketch.add(hash(row), row);
    }
    assert_eq!(sketch.samples.len(), DISTINCT_CAPACITY);
    assert!(sketch.candidates.len() <= CANDIDATE_CAPACITY);
    assert!(sketch.nominees().contains_key(&hash(0)));
    assert!((950_000..1_050_000).contains(&sketch.distinct()));
    // Counters lose mass during candidate selection; output counts must not.
    let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
        (0..50_000).map(|_| 0).chain(1..100_001),
    ));
    let selected = crate::col_view::test::Fixture::all(&array);
    let summary = analyze(selected.view()).unwrap();
    assert_eq!(
        (summary.values[0].label.as_str(), summary.values[0].count),
        ("0", 50_000)
    );
    let mut unique = Sketch::default();
    for row in 0..DISTINCT_CAPACITY {
        unique.add(hash(row), row);
    }
    assert!(!unique.overflowed);
    assert_eq!(unique.distinct(), DISTINCT_CAPACITY as u64);
    // Complete MG eviction cycles still have representative fallback values.
    let array: ArrayRef = Arc::new(Int64Array::from_iter_values(0..257 * 20));
    let selected = crate::col_view::test::Fixture::all(&array);
    let summary = analyze(selected.view()).unwrap();
    assert_eq!(summary.values.len(), OUTPUT_LIMIT);
    assert!(summary.values.iter().all(|value| value.count == 1));
}

#[test]
fn native_hashing_respects_float_and_dictionary_value_identity() {
    let array: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(f64::from_bits(0x7ff8000000000001)),
        None,
    ]));
    let all = crate::col_view::test::Fixture::all(&array);
    let summary = analyze(all.view()).unwrap();
    assert_eq!(summary.distinct_count, 2);
    assert!(summary.values.iter().all(|value| value.count == 2));
    let dictionary: ArrayRef = Arc::new(
        DictionaryArray::<Int32Type>::try_new(
            Int32Array::from(vec![Some(0), Some(1), Some(2), None, Some(3)]),
            Arc::new(StringArray::from(vec![
                Some("same"),
                Some("same"),
                None,
                Some("other"),
            ])),
        )
        .unwrap(),
    );
    let selected = crate::col_view::test::Fixture::new(&dictionary, vec![4, 2, 0, 1, 3]);
    let summary = analyze(selected.view()).unwrap();
    assert_eq!(summary.distinct_count, 2);
    assert_eq!(
        (summary.values[0].label.as_str(), summary.values[0].count),
        ("same", 2)
    );
}

#[test]
fn nested_dictionary_hashes_value_identity_and_skips_nulls_at_each_level() {
    let values = Arc::new(StringArray::from(vec![
        Some("one"),
        None,
        Some("one"),
        Some("two"),
    ]));
    // More entries than the compact code space force the hash-based analysis.
    let entries = Int32Array::from_iter((0..300).map(|key| match key {
        0..=2 => Some(key),
        3 => None,
        _ => Some(3),
    }));
    let inner = Arc::new(DictionaryArray::<Int32Type>::new(entries, values));
    let keys = Int32Array::from(vec![Some(0), Some(1), Some(2), Some(3), None, Some(299)]);
    let nested: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::new(keys, inner));

    let all = crate::col_view::test::Fixture::all(&nested);
    let selected = crate::col_view::test::Fixture::masked(&nested, vec![0, 1, 2, 3, 4]);
    let one = ValueFrequency {
        label: "one".into(),
        count: 2,
    };
    let two = ValueFrequency {
        label: "two".into(),
        count: 1,
    };
    assert_eq!(analyze(all.view()).unwrap(), ValueSummary {
        distinct_count: 2,
        values: vec![one.clone(), two],
    });
    assert_eq!(analyze(selected.view()).unwrap(), ValueSummary {
        distinct_count: 1,
        values: vec![one],
    });
}

/// Columns above the code-space limit take the hashing path. With fewer
/// than 4,096 distinct values it is exact, so it must agree with the
/// encoded path's semantics for every supported storage type.
#[test]
fn hashing_path_counts_every_storage_type_like_the_encoded_path() {
    use arrow::array::{
        BooleanArray, Decimal128Array, DurationSecondArray, Float32Array, LargeStringArray,
        StringViewArray, TimestampMillisecondArray, UInt16Array,
    };
    const DISTINCT: usize = 300;
    // Value `i % DISTINCT`, plus four extra rows of value 7, plus a null.
    let ids: Vec<Option<usize>> = (0..2 * DISTINCT)
        .map(|i| Some(i % DISTINCT))
        .chain([Some(7); 4])
        .chain([None])
        .collect();
    let text = |id: usize| format!("v{id:03}");
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from_iter(
            ids.iter().map(|id| id.map(|id| id as i64)),
        )),
        Arc::new(Float32Array::from_iter(
            ids.iter().map(|id| id.map(|id| id as f32)),
        )),
        // Signed zeros and NaN payloads are one value each.
        Arc::new(Float64Array::from_iter(ids.iter().enumerate().map(
            |(row, id)| {
                id.map(|id| match (id, row < DISTINCT) {
                    (0, true) => -0.0,
                    (0, false) => 0.0,
                    (1, true) => f64::NAN,
                    (1, false) => f64::from_bits(0x7ff8_0000_0000_0001),
                    _ => id as f64,
                })
            },
        ))),
        Arc::new(
            Decimal128Array::from_iter(ids.iter().map(|id| id.map(|id| id as i128)))
                .with_precision_and_scale(10, 2)
                .unwrap(),
        ),
        Arc::new(DurationSecondArray::from_iter(
            ids.iter().map(|id| id.map(|id| id as i64)),
        )),
        Arc::new(TimestampMillisecondArray::from_iter(
            ids.iter().map(|id| id.map(|id| id as i64)),
        )),
        Arc::new(StringArray::from_iter(ids.iter().map(|id| id.map(text)))),
        Arc::new(LargeStringArray::from_iter(
            ids.iter().map(|id| id.map(text)),
        )),
        Arc::new(StringViewArray::from_iter(
            ids.iter().map(|id| id.map(text)),
        )),
        Arc::new(arrow::array::Float16Array::from_iter(
            ids.iter()
                .map(|id| id.map(|id| half::f16::from_f32(id as f32))),
        )),
        Arc::new(
            arrow::array::Decimal256Array::from_iter(
                ids.iter().map(|id| id.map(|id| i256::from(id as i64))),
            )
            .with_precision_and_scale(40, 1)
            .unwrap(),
        ),
        Arc::new(arrow::array::DurationMillisecondArray::from_iter(
            ids.iter().map(|id| id.map(|id| id as i64)),
        )),
        // A native dictionary with more entries than the compact code space:
        // keys repeat entries, and two entries share one value.
        Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                Int32Array::from_iter(ids.iter().map(|id| id.map(|id| id as i32))),
                Arc::new(StringArray::from_iter_values(
                    (0..DISTINCT).map(|id| text(if id == 299 { 298 } else { id })),
                )),
            )
            .unwrap(),
        ),
    ];
    for array in arrays {
        let expected_distinct = match array.data_type() {
            DataType::Dictionary(..) => DISTINCT as u64 - 1,
            _ => DISTINCT as u64,
        };
        let all = crate::col_view::test::Fixture::all(&array);
        let summary = analyze(all.view()).unwrap();
        assert_eq!(
            summary.distinct_count,
            expected_distinct,
            "{}",
            array.data_type()
        );
        assert_eq!(summary.values[0].count, 6, "{}", array.data_type());
        assert_eq!(summary.values.len(), OUTPUT_LIMIT);
        // Explicit rows, including the null row, scan only the selection.
        let null_row = ids.len() as u32 - 1;
        let selected = crate::col_view::test::Fixture::new(&array, vec![null_row, 7, 307, 8]);
        let summary = analyze(selected.view()).unwrap();
        assert_eq!(summary.distinct_count, 2, "{}", array.data_type());
        assert_eq!(summary.values[0].count, 2, "{}", array.data_type());
        assert_eq!(summary.values[1].count, 1, "{}", array.data_type());
    }
    // A native dictionary of booleans larger than the code space hashes
    // its two values once per entry.
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int32Type>::try_new(
            Int32Array::from_iter_values((0..600).map(|i| i % 300)),
            Arc::new(BooleanArray::from_iter((0..300).map(|i| Some(i % 3 == 0)))),
        )
        .unwrap(),
    );
    let summary = analyze(crate::col_view::test::Fixture::all(&array).view()).unwrap();
    assert_eq!(summary.distinct_count, 2);
    assert_eq!(
        (summary.values[0].label.as_str(), summary.values[0].count),
        ("false", 400)
    );
    // Low-cardinality storage types use exact codes, also over explicit
    // rows with nulls.
    let array: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        None,
        Some(false),
        Some(true),
    ]));
    let selected = crate::col_view::test::Fixture::new(&array, vec![3, 1, 0]);
    let summary = analyze(selected.view()).unwrap();
    assert_eq!(summary.distinct_count, 1);
    assert_eq!(
        (summary.values[0].label.as_str(), summary.values[0].count),
        ("true", 2)
    );
    let array: ArrayRef = Arc::new(UInt16Array::from(vec![Some(1), None, Some(1)]));
    assert_eq!(
        analyze(crate::col_view::test::Fixture::all(&array).view())
            .unwrap()
            .values[0]
            .count,
        2
    );
}
