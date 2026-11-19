#![cfg(feature = "parquet")]
use std::sync::Arc;

use arrow::array::{
    ArrayRef, Decimal128Array, Float64Array, Int32Array, RecordBatch, StringArray,
    TimestampNanosecondArray, UInt64Array,
};
use gewu_core::{AxisScale, ColumnFilter, ColumnId, Error, Filter};
use parquet::arrow::ArrowWriter;

fn document(columns: Vec<(&str, ArrayRef)>) -> Arc<gewu_core::Dataset> {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Arc::new(gewu_core::Dataset::open(file.path()).unwrap())
}

#[test]
fn capabilities_are_static_and_choices_keep_only_visible_values() {
    let doc = document(vec![
        (
            "category",
            Arc::new(Int32Array::from_iter_values((0..600).map(|i| i % 3))),
        ),
        ("id", Arc::new(Int32Array::from_iter_values(0..600))),
        (
            "text",
            Arc::new(StringArray::from_iter_values(
                (0..600).map(|i| format!("value-{i}")),
            )),
        ),
    ]);
    let (c0, c1) = (ColumnId::from_raw(0), ColumnId::from_raw(1));
    let original = doc.unfiltered().filter_choices(c0).unwrap().values.unwrap();
    assert_eq!(original.iter().map(|v| v.count).collect::<Vec<_>>(), [
        200, 200, 200
    ]);
    assert!(doc
        .unfiltered()
        .filter_choices(c1)
        .unwrap()
        .values
        .is_none());
    assert!(doc.info().columns[1].capabilities.range);
    assert!(doc.info().columns[1].capabilities.exact);
    assert!(doc.info().columns[2].capabilities.text_match);
    assert!(doc.info().columns[2].capabilities.exact);
    let values = doc
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::range(None, Some("0".into()), false),
        }])
        .unwrap()
        .filter_choices(c0)
        .unwrap()
        .values
        .unwrap();
    assert_eq!(values.iter().map(|v| v.count).collect::<Vec<_>>(), [1]);
    assert_eq!(values.iter().map(|v| &v.value).collect::<Vec<_>>(), vec![
        &original[0].value
    ]);
    let chosen = doc
        .filter(&[ColumnFilter {
            column: c0,
            filter: Filter::exact(vec![values[0].value.clone()], false, false),
        }])
        .unwrap();
    assert_eq!(chosen.row_count(), 200);
    assert_eq!(chosen.filter_choices(c0).unwrap().values.unwrap(), vec![
        original[0].clone()
    ]);
}

#[test]
fn provided_bin_bounds_round_trip_above_double_integer_precision() {
    let base = u64::MAX - 599;
    let doc = document(vec![(
        "id",
        Arc::new(UInt64Array::from_iter_values(base..=u64::MAX)),
    )]);
    let c0 = ColumnId::from_raw(0);
    let distribution = doc
        .unfiltered()
        .filter_distribution(c0, 60, AxisScale::Linear)
        .unwrap()
        .unwrap();
    for (bin, bounds) in distribution.ranges.iter().enumerate() {
        let view = doc
            .filter(&[ColumnFilter {
                column: c0,
                filter: Filter::range(
                    bounds.min.as_ref().map(|b| b.value.clone()),
                    bounds.max.as_ref().map(|b| b.value.clone()),
                    false,
                ),
            }])
            .unwrap();
        assert_eq!(view.row_count(), distribution.histogram.counts[bin]);
    }
}

#[test]
fn invalid_text_leaves_the_applied_view_intact() {
    let doc = document(vec![("id", Arc::new(Int32Array::from(vec![1, 2, 3])))]);
    let c0 = ColumnId::from_raw(0);
    let before = doc.unfiltered().unsorted();
    assert!(matches!(
        doc.filter(&[ColumnFilter {
            column: c0,
            filter: Filter::range(Some("invalid".into()), None, false),
        }]),
        Err(Error::Filter(gewu_core::FilterError::Bound(
            gewu_core::BoundError::InvalidValue { .. }
        )))
    ));
    assert_eq!(
        doc.unfiltered()
            .unsorted()
            .rows(0, 10, &[c0])
            .unwrap()
            .cells,
        before.rows(0, 10, &[c0]).unwrap().cells
    );
}

#[test]
fn float_choices_group_signed_zero_and_nan_payloads_with_nulls_separate() {
    let doc = document(vec![(
        "score",
        Arc::new(Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(1.0),
            None,
        ])),
    )]);
    let c0 = ColumnId::from_raw(0);
    let choices = doc.unfiltered().filter_choices(c0).unwrap();
    assert_eq!(choices.null_count, 1);
    let values = choices.values.unwrap();
    assert_eq!(values.iter().map(|v| v.count).collect::<Vec<_>>(), [
        2, 1, 2
    ]);
    for value in values {
        assert_eq!(
            doc.filter(&[ColumnFilter {
                column: c0,
                filter: Filter::exact(vec![value.value], false, false),
            }])
            .unwrap()
            .row_count(),
            value.count
        );
    }
}

#[test]
fn bounded_choices_and_empty_columns_have_explicit_results() {
    let c0 = ColumnId::from_raw(0);
    for (count, bounded) in [(255, true), (256, false)] {
        let doc = document(vec![(
            "value",
            Arc::new(Int32Array::from_iter((0..count).map(Some).chain([None]))),
        )]);
        let choices = doc.unfiltered().filter_choices(c0).unwrap();
        assert_eq!(choices.null_count, 1);
        assert_eq!(choices.values.is_some(), bounded);
    }
    let doc = document(vec![(
        "value",
        Arc::new(Int32Array::from(Vec::<i32>::new())),
    )]);
    assert!(doc
        .unfiltered()
        .filter_choices(c0)
        .unwrap()
        .values
        .unwrap()
        .is_empty());
}

#[test]
fn high_cardinality_exact_text_filters_do_not_require_a_value_index() {
    let doc = document(vec![(
        "text",
        Arc::new(StringArray::from_iter_values(
            (0..100_001).map(|i| format!("value-{i:06}")),
        )),
    )]);
    let c0 = ColumnId::from_raw(0);
    assert!(doc
        .unfiltered()
        .filter_choices(c0)
        .unwrap()
        .values
        .is_none());
    let view = doc
        .filter(&[ColumnFilter {
            column: c0,
            filter: Filter::exact(vec!["value-100000".into()], false, false),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(view.rows(0, 10, &[c0]).unwrap().source_rows, [100_000]);
}

#[test]
fn exact_values_and_captured_facets_keep_byte_and_null_semantics() {
    let doc = document(vec![
        (
            "text",
            Arc::new(StringArray::from(vec![
                None,
                Some(""),
                Some("cat"),
                Some("Cat"),
                Some("cat"),
            ])),
        ),
        ("group", Arc::new(Int32Array::from(vec![1, 1, 1, 1, 2]))),
    ]);
    let (c0, c1) = (ColumnId::from_raw(0), ColumnId::from_raw(1));
    let filtered = doc
        .filter(&[
            ColumnFilter {
                column: c0,
                filter: Filter::exact(vec!["Cat".into()], false, false),
            },
            ColumnFilter {
                column: c1,
                filter: Filter::exact(vec!["1".into()], false, false),
            },
        ])
        .unwrap();
    let choices = filtered.filter_choices(c0).unwrap();
    assert_eq!(
        choices
            .values
            .unwrap()
            .iter()
            .map(|v| (v.value.as_str(), v.count))
            .collect::<Vec<_>>(),
        [("Cat", 1)]
    );
    assert_eq!(filtered.filter_choices(c0).unwrap().null_count, 0);
    for (values, exclude, include_nulls, expected) in [
        (vec!["cat".into()], false, true, vec![0, 2, 4]),
        (vec!["cat".into()], true, false, vec![1, 3]),
        (vec![], false, false, vec![]),
        (vec![], false, true, vec![0]),
        (vec![], true, false, vec![1, 2, 3, 4]),
        (vec![], true, true, vec![0, 1, 2, 3, 4]),
    ] {
        let view = doc
            .filter(&[ColumnFilter {
                column: c0,
                filter: Filter::exact(values, exclude, include_nulls),
            }])
            .unwrap()
            .unsorted();
        assert_eq!(view.rows(0, 10, &[c0]).unwrap().source_rows, expected);
    }
}

#[test]
fn candidate_values_round_trip_large_integers_decimals_and_zoned_timestamps() {
    let doc = document(vec![
        (
            "id",
            Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX])),
        ),
        (
            "decimal",
            Arc::new(
                Decimal128Array::from(vec![123456789012345678901_i128, 123456789012345678902])
                    .with_precision_and_scale(30, 12)
                    .unwrap(),
            ),
        ),
        (
            "time",
            Arc::new(
                TimestampNanosecondArray::from(vec![1735689600000000001_i64, 1735689600000000002])
                    .with_timezone("Europe/London"),
            ),
        ),
    ]);
    for col_idx in 0..3 {
        let column = ColumnId::from_raw(col_idx);
        for (row, value) in doc
            .unfiltered()
            .filter_choices(column)
            .unwrap()
            .values
            .unwrap()
            .iter()
            .enumerate()
        {
            let view = doc
                .filter(&[ColumnFilter {
                    column,
                    filter: Filter::exact(vec![value.value.clone()], false, false),
                }])
                .unwrap()
                .unsorted();
            assert_eq!(view.rows(0, 10, &[column]).unwrap().source_rows, [
                row as u64
            ]);
        }
    }
}

#[test]
fn exact_selection_keeps_null_policy_without_a_bounded_dictionary() {
    let doc = document(vec![(
        "value",
        Arc::new(Int32Array::from_iter((0..300).map(Some).chain([None]))),
    )]);
    let c0 = ColumnId::from_raw(0);
    assert!(doc
        .unfiltered()
        .filter_choices(c0)
        .unwrap()
        .values
        .is_none());
    for (exclude, include_nulls, count) in [
        (false, false, 2),
        (false, true, 3),
        (true, false, 298),
        (true, true, 299),
    ] {
        let view = doc
            .filter(&[ColumnFilter {
                column: c0,
                filter: Filter::exact(vec!["1".into(), "299".into()], exclude, include_nulls),
            }])
            .unwrap();
        assert_eq!(view.row_count(), count);
    }
}
