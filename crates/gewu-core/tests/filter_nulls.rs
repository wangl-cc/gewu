#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use gewu_core::{Document, Filter, FilterEditor, ViewState};
use parquet::arrow::ArrowWriter;

fn document() -> Document {
    let score: ArrayRef = Arc::new(Float64Array::from(vec![
        None,
        Some(1.0),
        Some(2.0),
        Some(3.0),
        None,
        Some(f64::NAN),
        Some(2.0),
    ]));
    let text: ArrayRef = Arc::new(StringArray::from(vec![
        None,
        Some("alpha"),
        Some("beta"),
        Some("alphabet"),
        Some("alpha"),
        Some("alphabet"),
        None,
    ]));
    let category: ArrayRef = Arc::new(StringArray::from(vec!["A", "A", "B", "A", "B", "A", "A"]));
    let marker: ArrayRef = Arc::new(StringArray::from(vec![
        "needle", "needle", "needle", "other", "needle", "needle", "other",
    ]));
    let batch = RecordBatch::try_from_iter(vec![
        ("score", score),
        ("text", text),
        ("category", category),
        ("marker", marker),
    ])
    .unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Document::open(file.path()).unwrap()
}

fn range(doc: &Document, include_nulls: bool) -> Filter {
    Filter::Range {
        column: 0,
        min: Some(doc.parse_bound(0, "1").unwrap()),
        max: Some(doc.parse_bound(0, "2").unwrap()),
        include_nulls,
    }
}

fn contains(include_nulls: bool) -> Filter {
    Filter::Contains {
        column: 1,
        text: "ALPHA".into(),
        case_sensitive: false,
        include_nulls,
    }
}

fn rows(doc: &Document, state: ViewState) -> Vec<u64> {
    let view = doc.apply_view(&state).unwrap();
    doc.rows(view.version, 0, 20, &[0, 1]).unwrap().source_rows
}

#[test]
fn range_null_policy_keeps_nulls_without_matching_nan_or_outside_values() {
    let doc = document();
    for (filter, expected) in [
        (range(&doc, false), vec![1, 2, 6]),
        (range(&doc, true), vec![0, 1, 2, 4, 6]),
        (
            Filter::IsNull {
                column: 0,
                negate: false,
            },
            vec![0, 4],
        ),
        (
            Filter::IsNull {
                column: 0,
                negate: true,
            },
            vec![1, 2, 3, 5, 6],
        ),
    ] {
        assert_eq!(
            rows(&doc, ViewState {
                filters: vec![filter],
                ..Default::default()
            }),
            expected
        );
    }
}

#[test]
fn contains_null_policy_preserves_case_matching_and_only_nulls() {
    let doc = document();
    for (filter, expected) in [
        (contains(false), vec![1, 3, 4, 5]),
        (contains(true), vec![0, 1, 3, 4, 5, 6]),
        (
            Filter::Contains {
                column: 1,
                text: "ALPHA".into(),
                case_sensitive: true,
                include_nulls: true,
            },
            vec![0, 6],
        ),
        (
            Filter::IsNull {
                column: 1,
                negate: false,
            },
            vec![0, 6],
        ),
    ] {
        assert_eq!(
            rows(&doc, ViewState {
                filters: vec![filter],
                ..Default::default()
            }),
            expected
        );
    }
}

#[test]
fn included_nulls_still_obey_other_columns_and_global_search() {
    let doc = document();
    for (filter, expected) in [
        (range(&doc, true), vec![0, 1]),
        (contains(true), vec![0, 1, 5]),
    ] {
        assert_eq!(
            rows(&doc, ViewState {
                filters: vec![filter, Filter::Equals {
                    column: 2,
                    value: "A".into(),
                }],
                search: Some("needle".into()),
                ..Default::default()
            }),
            expected
        );
    }
}

#[test]
fn facets_count_included_nulls_and_still_ignore_only_their_own_filter() {
    let doc = document();
    let state = ViewState {
        filters: vec![range(&doc, true), Filter::Equals {
            column: 2,
            value: "A".into(),
        }],
        search: Some("needle".into()),
        ..Default::default()
    };
    let values = doc
        .prepare_text_choices(2, Some(&state))
        .unwrap()
        .page("", None, 100)
        .unwrap()
        .values;
    assert_eq!(
        values
            .iter()
            .map(|value| (value.value.as_deref(), value.count))
            .collect::<Vec<_>>(),
        [(Some("A"), 2), (Some("B"), 2)]
    );

    // Removing only score's own filter leaves source rows 0, 1 and 5: null,
    // one finite value, and NaN. The UI must not conflate those last two classes.
    let FilterEditor::Choices { values } = doc.filter_spec(0, Some(&state)).unwrap().editor else {
        panic!("small numeric column should offer choices")
    };
    assert_eq!(values.iter().map(|value| value.count).sum::<u64>(), 3);
    assert_eq!(
        values
            .iter()
            .find(|value| value.label.is_none())
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        values
            .iter()
            .find(|value| value.label.as_deref() == Some("NaN"))
            .unwrap()
            .count,
        1
    );
}
