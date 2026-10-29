//! Behaviour the refactor must keep: which rows facets count, versioning
//! and cancellation.

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Bound, Document, Error, Filter, ViewState};
use parquet::arrow::ArrowWriter;

/// name | label | price | id
fn sample(tag: &str) -> (PathBuf, Document) {
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
    write(tag, &batch)
}

fn write(tag: &str, batch: &RecordBatch) -> (PathBuf, Document) {
    let path = std::env::temp_dir().join(format!(
        "gewu-facets-{tag}-{}-{:?}.parquet",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(batch).unwrap();
    w.close().unwrap();
    let doc = Document::open(&path).unwrap();
    (path, doc)
}

fn labels_of(doc: &Document, column: u32, state: Option<&ViewState>) -> Vec<(String, u64)> {
    doc.distinct_values(column, state, 100)
        .unwrap()
        .values
        .into_iter()
        .map(|v| (v.label.unwrap_or_else(|| "<null>".into()), v.count))
        .collect()
}

/// label in {cat, dog}, price >= 2, and a search for "a" (names a only;
/// labels cat; no prices or ids contain it as a value).
fn busy_state(doc: &Document) -> ViewState {
    let keys = doc
        .keys_for_labels(1, &[Some("cat".into()), Some("dog".into())])
        .unwrap();
    ViewState {
        filters: vec![Filter::InValues { column: 1, keys }, Filter::Range {
            column: 2,
            min: Some(Bound::Float(2.0)),
            max: None,
        }],
        search: None,
        ..Default::default()
    }
}

#[test]
fn distinct_ignores_only_its_own_column() {
    let (path, doc) = sample("distinct");
    let state = busy_state(&doc);

    // label: only the price filter applies -> rows a, c, e, f
    assert_eq!(labels_of(&doc, 1, Some(&state)), [
        ("<null>".into(), 1),
        ("bird".into(), 1),
        ("cat".into(), 1),
        ("dog".into(), 1),
    ]);
    // price: only the label filter applies -> rows a, b, d, f
    let prices: Vec<String> = labels_of(&doc, 2, Some(&state))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(prices, ["<null>", "1.0", "3.5", "7.0"]);
    // name: both filters apply -> rows a, f
    let names: Vec<String> = labels_of(&doc, 0, Some(&state))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(names, ["a", "f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn distinct_and_histogram_apply_the_search() {
    let (path, doc) = sample("search");
    // "cat" matches labels of rows a and d.
    let state = ViewState {
        search: Some("cat".into()),
        ..Default::default()
    };
    let names: Vec<String> = labels_of(&doc, 0, Some(&state))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(names, ["a", "d"]);

    let h = doc.histogram(3, Some(&state), 6).unwrap();
    assert_eq!(h.total, 2);
    assert_eq!(h.counts.iter().sum::<u64>(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_ignores_only_its_own_column() {
    let (path, doc) = sample("hist");
    let mut state = busy_state(&doc);
    state.filters.push(Filter::Range {
        column: 3,
        min: Some(Bound::Int(60)),
        max: None,
    });
    // id: label and price filters apply -> rows a, f; its own range does not.
    let h = doc.histogram(3, Some(&state), 6).unwrap();
    assert_eq!(h.total, 2);
    // price: label and id filters apply -> row f only.
    let h = doc.histogram(2, Some(&state), 4).unwrap();
    assert_eq!(h.total, 1);
    // Axis always spans the whole column.
    assert_eq!(h.lo, Some(1.0));
    assert_eq!(h.hi, Some(9.0));
    std::fs::remove_file(path).ok();
}

#[test]
fn view_histogram_and_summary_follow_the_current_view() {
    let (path, doc) = sample("view");
    let state = busy_state(&doc);
    let v = doc.apply_view(&state).unwrap();
    assert_eq!(v.row_count, 2); // a, f

    let h = doc.view_histogram(v.version, 3, 6).unwrap();
    assert_eq!(h.total, 2);
    let s = doc.column_summary(2, 5).unwrap();
    assert_eq!(s.rows, 2);
    assert_eq!(s.min, Some(3.5));
    assert_eq!(s.max, Some(7.0));

    doc.apply_view(&ViewState::default()).unwrap();
    assert!(matches!(
        doc.view_histogram(v.version, 3, 6),
        Err(Error::StaleVersion { .. })
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn failed_apply_keeps_the_old_view() {
    let (path, doc) = sample("failed");
    let before = doc.current_view();
    let bad = ViewState {
        filters: vec![Filter::Range {
            column: 0,
            min: Some(Bound::Int(1)),
            max: None,
        }],
        ..Default::default()
    };
    assert!(doc.apply_view(&bad).is_err());
    assert_eq!(doc.current_view(), before);
    std::fs::remove_file(path).ok();
}

#[test]
fn unknown_column_is_an_argument_error() {
    let (path, doc) = sample("unknown");
    assert!(matches!(
        doc.distinct_values(9, None, 10),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        doc.histogram(9, None, 10),
        Err(Error::InvalidArgument(_))
    ));
    let v = doc.current_view();
    assert!(matches!(
        doc.rows(v.version, 0, 1, &[9]),
        Err(Error::InvalidArgument(_))
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn cancelled_apply_never_replaces_the_view() {
    // Enough rows that applying a view takes a moment.
    let n = 2_000_000;
    let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(0..n));
    let batch = RecordBatch::try_from_iter(vec![("id", ids)]).unwrap();
    let (path, doc) = write("cancel", &batch);
    let doc = Arc::new(doc);
    let before = doc.current_view();

    let state = ViewState {
        search: Some("12345".into()),
        sort: vec![gewu_core::SortKey {
            column: 0,
            descending: true,
        }],
        ..Default::default()
    };
    let worker = {
        let doc = doc.clone();
        std::thread::spawn(move || doc.apply_view(&state))
    };
    doc.cancel_pending();
    match worker.join().unwrap() {
        // Finished before the cancel landed: it committed a new view.
        Ok(info) => assert_eq!(doc.current_view(), info),
        Err(Error::Cancelled) => assert_eq!(doc.current_view(), before),
        Err(e) => panic!("unexpected error {e}"),
    }
    std::fs::remove_file(path).ok();
}
