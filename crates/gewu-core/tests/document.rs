use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Document, Error, Filter, SortKey, ViewState};
use parquet::arrow::ArrowWriter;

/// name | label | price
fn sample() -> (PathBuf, Document) {
    let names: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "c", "d", "e", "f"]));
    let labels: ArrayRef = Arc::new(StringArray::from(vec![
        Some("cat"),
        Some("dog"),
        None,
        Some("cat"),
        Some("bird"),
        Some("dog"),
    ]));
    let prices: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(3.5),
        Some(1.0),
        Some(9.0),
        None,
        Some(2.25),
        Some(7.0),
    ]));
    let ids: ArrayRef = Arc::new(Int64Array::from(vec![10, 20, 30, 40, 50, 60]));
    let batch = RecordBatch::try_from_iter(vec![
        ("name", names),
        ("label", labels),
        ("price", prices),
        ("id", ids),
    ])
    .unwrap();

    let path = std::env::temp_dir().join(format!(
        "gewu-test-{}-{:?}.parquet",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(&batch).unwrap();
    w.close().unwrap();
    let doc = Document::open(&path).unwrap();
    (path, doc)
}

fn names(doc: &Document) -> Vec<String> {
    let v = doc.current_view();
    let block = doc.rows(v.version, 0, 100, &[0]).unwrap();
    (0..block.n_rows)
        .map(|r| block.cell(r, 0).unwrap().to_string())
        .collect()
}

#[test]
fn opens_file() {
    let (path, doc) = sample();
    let info = doc.info();
    assert_eq!(info.row_count, 6);
    assert_eq!(info.columns.len(), 4);
    assert_eq!(info.columns[1].name, "label");
    assert_eq!(doc.current_view().row_count, 6);
    assert_eq!(names(&doc), ["a", "b", "c", "d", "e", "f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn sorts_with_nulls_last() {
    let (path, doc) = sample();
    let state = ViewState {
        sort: vec![SortKey {
            column: 2,
            descending: false,
        }],
        ..Default::default()
    };
    doc.apply_view(&state).unwrap();
    assert_eq!(names(&doc), ["b", "e", "a", "f", "c", "d"]);

    let state = ViewState {
        sort: vec![SortKey {
            column: 2,
            descending: true,
        }],
        ..Default::default()
    };
    doc.apply_view(&state).unwrap();
    assert_eq!(names(&doc), ["c", "f", "a", "e", "b", "d"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn checkbox_filter_and_distinct() {
    let (path, doc) = sample();
    let all = doc.distinct_values(1, None, 100).unwrap();
    let labels: Vec<_> = all.values.iter().map(|v| v.label.clone()).collect();
    assert_eq!(
        labels,
        [
            None,
            Some("bird".into()),
            Some("cat".into()),
            Some("dog".into())
        ]
    );
    assert_eq!(all.values[2].count, 2);

    let keys = doc
        .keys_for_labels(1, &[Some("cat".into()), Some("dog".into())])
        .unwrap();
    let state = ViewState {
        filters: vec![Filter::InValues { column: 1, keys }],
        sort: vec![SortKey {
            column: 3,
            descending: true,
        }],
        search: None,
    };
    let v = doc.apply_view(&state).unwrap();
    assert_eq!(v.row_count, 4);
    assert_eq!(names(&doc), ["f", "d", "b", "a"]);

    // Distinct values of another column respect this filter.
    let prices = doc.distinct_values(2, Some(&state), 100).unwrap();
    assert_eq!(prices.total, 4);
    // ...but the filtered column itself still lists every value (like Excel).
    let labels = doc.distinct_values(1, Some(&state), 100).unwrap();
    assert_eq!(labels.total, 4);
    std::fs::remove_file(path).ok();
}

#[test]
fn null_contains_and_search() {
    let (path, doc) = sample();
    let state = ViewState {
        filters: vec![Filter::IsNull {
            column: 2,
            negate: false,
        }],
        ..Default::default()
    };
    doc.apply_view(&state).unwrap();
    assert_eq!(names(&doc), ["d"]);

    let state = ViewState {
        filters: vec![Filter::Contains {
            column: 1,
            text: "D".into(),
            case_sensitive: false,
        }],
        ..Default::default()
    };
    doc.apply_view(&state).unwrap();
    assert_eq!(names(&doc), ["b", "e", "f"]);

    let state = ViewState {
        search: Some("2.25".into()),
        ..Default::default()
    };
    doc.apply_view(&state).unwrap();
    assert_eq!(names(&doc), ["e"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn stale_version_is_rejected() {
    let (path, doc) = sample();
    let old = doc.current_view().version;
    doc.apply_view(&ViewState::default()).unwrap();
    assert!(matches!(
        doc.rows(old, 0, 10, &[0]),
        Err(Error::StaleVersion { .. })
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn stats() {
    let (path, doc) = sample();
    let s = doc.column_stats(2).unwrap();
    assert_eq!(s.null_count, 1);
    assert_eq!(s.distinct_count, 5);
    assert_eq!(s.min.as_deref(), Some("1.0"));
    assert_eq!(s.max.as_deref(), Some("9.0"));
    std::fs::remove_file(path).ok();
}
