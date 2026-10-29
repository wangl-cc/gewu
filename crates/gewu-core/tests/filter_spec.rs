#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Int32Array, RecordBatch, StringArray, UInt64Array},
    datatypes::DataType,
};
use gewu_core::{Bound, DataKind, Document, Error, Filter, FilterEditor, ViewState};
use parquet::arrow::ArrowWriter;

fn document(columns: Vec<(&str, ArrayRef)>) -> Document {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Document::open(file.path()).unwrap()
}

#[test]
fn source_cardinality_selects_editors_and_facet_counts_preserve_choices() {
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
    let FilterEditor::Choices { values: original } = doc.filter_spec(0, None).unwrap().editor
    else {
        panic!("low-cardinality numeric editor")
    };
    assert_eq!(original.iter().map(|v| v.count).collect::<Vec<_>>(), [
        200, 200, 200
    ]);
    assert!(matches!(
        doc.filter_spec(1, None).unwrap().editor,
        FilterEditor::Range {
            kind: DataKind::Integer,
            distribution: Some(_)
        }
    ));
    assert_eq!(
        doc.filter_spec(2, None).unwrap().editor,
        FilterEditor::Contains
    );
    let state = ViewState {
        filters: vec![Filter::Range {
            column: 1,
            min: None,
            max: Some(doc.make_bound_i64(1, 0).unwrap()),
        }],
        ..Default::default()
    };
    let FilterEditor::Choices { values } = doc.filter_spec(0, Some(&state)).unwrap().editor else {
        panic!("stable choices")
    };
    assert_eq!(values.iter().map(|v| v.count).collect::<Vec<_>>(), [
        1, 0, 0
    ]);
    assert_eq!(
        values.iter().map(|v| v.key).collect::<Vec<_>>(),
        original.iter().map(|v| v.key).collect::<Vec<_>>()
    );
    assert_eq!(
        doc.filter_spec(2, Some(&state)).unwrap().editor,
        FilterEditor::Contains
    );
    let chosen = ViewState {
        filters: vec![Filter::InValues {
            column: 0,
            keys: vec![values[1].key],
        }],
        ..Default::default()
    };
    assert_eq!(doc.apply_view(&chosen).unwrap().row_count, 200);
    let FilterEditor::Choices { values: own_facet } =
        doc.filter_spec(0, Some(&chosen)).unwrap().editor
    else {
        panic!("choices")
    };
    assert_eq!(
        own_facet, original,
        "the column's own filter does not hide its options"
    );
}

#[test]
fn provided_bin_bounds_round_trip_above_double_integer_precision() {
    let base = u64::MAX - 599;
    let doc = document(vec![(
        "id",
        Arc::new(UInt64Array::from_iter_values(base..=u64::MAX)),
    )]);
    let FilterEditor::Range {
        distribution: Some(distribution),
        ..
    } = doc.filter_spec(0, None).unwrap().editor
    else {
        panic!("range chart")
    };
    let mut all_rows = Vec::new();
    for (bounds, count) in distribution
        .ranges
        .into_iter()
        .zip(distribution.histogram.counts)
    {
        let view = doc
            .apply_view(&ViewState {
                filters: vec![Filter::Range {
                    column: 0,
                    min: bounds.min,
                    max: bounds.max,
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(view.row_count, count);
        all_rows.extend(doc.rows(view.version, 0, 600, &[0]).unwrap().source_rows);
    }
    assert_eq!(all_rows, (0..600).collect::<Vec<u64>>());
    let exact = doc.make_bound_u64(0, u64::MAX).unwrap();
    let view = doc
        .apply_view(&ViewState {
            filters: vec![Filter::Range {
                column: 0,
                min: Some(exact.clone()),
                max: Some(exact),
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(doc.rows(view.version, 0, 10, &[0]).unwrap().source_rows, [
        599
    ]);
}

#[test]
fn typed_bounds_reject_mismatched_columns_without_replacing_view() {
    let doc = document(vec![("id", Arc::new(Int32Array::from(vec![1, 2, 3])))]);
    let before = doc.current_view();
    let wrong = Bound::from_i64(&DataType::Int64, 2).unwrap();
    assert!(matches!(
        doc.apply_view(&ViewState {
            filters: vec![Filter::Range {
                column: 0,
                min: Some(wrong),
                max: None
            }],
            ..Default::default()
        }),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(doc.current_view(), before);
}

#[test]
fn choice_limit_counts_null_and_empty_columns_remain_valid() {
    for (count, choices) in [(499, true), (500, false)] {
        let array = Int32Array::from_iter((0..count).map(Some).chain([None]));
        let doc = document(vec![("value", Arc::new(array))]);
        let spec = doc.filter_spec(0, None).unwrap();
        assert!(spec.nullable);
        assert_eq!(matches!(spec.editor, FilterEditor::Choices { .. }), choices);
    }
    let doc = document(vec![(
        "value",
        Arc::new(Int32Array::from(Vec::<i32>::new())),
    )]);
    assert!(
        matches!(doc.filter_spec(0, None).unwrap().editor, FilterEditor::Choices { values } if values.is_empty())
    );
}

#[test]
fn bounds_can_restore_exact_bins_independently_of_text_and_axis_spelling() {
    let ty = DataType::UInt64;
    let first = Bound::from_u64(&ty, u64::MAX - 1).unwrap();
    let last = Bound::from_u64(&ty, u64::MAX).unwrap();
    assert_eq!(first.axis_value(), last.axis_value());
    assert!(!first.same_value(&last));
    let text = Bound::parse(&ty, "18446744073709551615.0").unwrap();
    assert_ne!(last.display(), text.display());
    assert!(last.same_value(&text));
    assert!(!last.same_value(&Bound::from_i64(&DataType::Int64, 0).unwrap()));
}
