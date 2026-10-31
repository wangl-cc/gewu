#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Int32Array, RecordBatch, StringArray, UInt64Array},
    datatypes::DataType,
};
use gewu_core::{
    Bound, Document, Error, Filter, FilterEditor, LogicalType, TextFilterMode, ViewState,
};
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
            logical_type: LogicalType::Integer {
                bits: 32,
                signed: true,
            },
            distribution: Some(_)
        }
    ));
    assert_eq!(
        doc.filter_spec(2, None).unwrap().editor,
        FilterEditor::Text {
            modes: vec![TextFilterMode::Choices, TextFilterMode::Contains],
            recommended: TextFilterMode::Contains
        }
    );
    let state = ViewState {
        filters: vec![Filter::Range {
            include_nulls: false,
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
        FilterEditor::Text {
            modes: vec![TextFilterMode::Choices, TextFilterMode::Contains],
            recommended: TextFilterMode::Contains
        }
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
    // The unit tests cover all cuts; here exercise the editor -> view -> rows path.
    let bin = distribution.ranges.len() / 2;
    let bounds = &distribution.ranges[bin];
    let view = doc
        .apply_view(&ViewState {
            filters: vec![Filter::Range {
                include_nulls: false,
                column: 0,
                min: bounds.min.clone(),
                max: bounds.max.clone(),
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(view.row_count, distribution.histogram.counts[bin]);
    let rows = doc.rows(view.version, 0, 600, &[0]).unwrap();
    let first = bounds
        .min
        .as_ref()
        .unwrap()
        .display()
        .parse::<u64>()
        .unwrap()
        - base;
    let last = bounds
        .max
        .as_ref()
        .unwrap()
        .display()
        .parse::<u64>()
        .unwrap()
        - base;
    assert_eq!(rows.source_rows, (first..=last).collect::<Vec<_>>());
}

#[test]
fn typed_bounds_reject_mismatched_columns_without_replacing_view() {
    let doc = document(vec![("id", Arc::new(Int32Array::from(vec![1, 2, 3])))]);
    let before = doc.current_view();
    let wrong = Bound::from_i64(&DataType::Int64, 2).unwrap();
    assert!(matches!(
        doc.apply_view(&ViewState {
            filters: vec![Filter::Range {
                include_nulls: false,
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
        let text = StringArray::from_iter((0..count).map(|i| Some(i.to_string())).chain([None]));
        let doc = document(vec![("text", Arc::new(text))]);
        assert_eq!(
            doc.filter_spec(0, None).unwrap().editor,
            FilterEditor::Text {
                modes: vec![TextFilterMode::Choices, TextFilterMode::Contains],
                recommended: if choices {
                    TextFilterMode::Choices
                } else {
                    TextFilterMode::Contains
                },
            }
        );
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
fn high_cardinality_text_choices_do_not_require_a_value_index() {
    let doc = document(vec![(
        "text",
        Arc::new(StringArray::from_iter_values(
            (0..100_001).map(|i| format!("value-{i:06}")),
        )),
    )]);
    let choices = doc
        .prepare_text_choices(0, Some(&ViewState::default()))
        .unwrap();
    let page = choices.page("value-100000", None, 100).unwrap();
    assert_eq!(page.values.len(), 1);
    assert_eq!(page.values[0].count, 1);
    let view = doc
        .apply_view(&ViewState {
            filters: vec![Filter::TextValues {
                column: 0,
                values: vec![page.values[0].value.clone()],
                exclude: false,
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(doc.rows(view.version, 0, 10, &[0]).unwrap().source_rows, [
        100_000
    ]);
}

#[test]
fn text_selection_and_captured_facets_keep_exact_value_semantics() {
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
    let state = ViewState {
        filters: vec![
            Filter::TextValues {
                column: 0,
                values: vec![Some("Cat".into())],
                exclude: false,
            },
            Filter::Equals {
                column: 1,
                value: "1".into(),
            },
        ],
        search: Some("cat".into()),
        ..Default::default()
    };
    let choices = doc.prepare_text_choices(0, Some(&state)).unwrap();
    doc.apply_view(&ViewState::default()).unwrap();
    assert_eq!(
        choices
            .page("", None, 10)
            .unwrap()
            .values
            .iter()
            .map(|v| (v.value.as_deref(), v.count))
            .collect::<Vec<_>>(),
        [(None, 0), (Some(""), 0), (Some("Cat"), 1), (Some("cat"), 1)]
    );
    for (values, exclude, expected) in [
        (vec![None, Some("cat".into())], false, vec![0, 2, 4]),
        (vec![None, Some("cat".into())], true, vec![1, 3]),
        (vec![], false, vec![]),
        (vec![], true, vec![0, 1, 2, 3, 4]),
    ] {
        let view = doc
            .apply_view(&ViewState {
                filters: vec![Filter::TextValues {
                    column: 0,
                    values,
                    exclude,
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            doc.rows(view.version, 0, 10, &[0]).unwrap().source_rows,
            expected
        );
    }
    choices.cancel();
    assert!(matches!(choices.page("", None, 10), Err(Error::Cancelled)));
    assert_eq!(
        doc.prepare_text_choices(0, None)
            .unwrap()
            .page("", None, 10)
            .unwrap()
            .values
            .len(),
        4
    );
    assert!(matches!(
        doc.prepare_text_choices(1, None),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        doc.validate_filter(&Filter::TextValues {
            column: 1,
            values: vec![],
            exclude: false
        }),
        Err(Error::InvalidArgument(_))
    ));
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
