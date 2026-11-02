#![cfg(feature = "parquet")]

//! Behaviour the refactor must keep: which rows facets count, versioning
//! and cancellation.

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Error, Filter, QuerySpec, Session};
use parquet::arrow::ArrowWriter;

/// name | label | price | id
fn sample(tag: &str) -> (PathBuf, Session) {
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

fn write(tag: &str, batch: &RecordBatch) -> (PathBuf, Session) {
    let path = std::env::temp_dir().join(format!(
        "gewu-facets-{tag}-{}-{:?}.parquet",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(batch).unwrap();
    w.close().unwrap();
    let doc = Session::open(&path).unwrap();
    (path, doc)
}

fn labels_of(doc: &Session, column: u32, spec: Option<&QuerySpec>) -> Vec<(String, u64)> {
    doc.distinct_values(column, spec, 100)
        .unwrap()
        .values
        .into_iter()
        .map(|v| (v.label.unwrap_or_else(|| "<null>".into()), v.count))
        .collect()
}

/// label in {cat, dog}, price >= 2, and a search for "a" (names a only;
/// labels cat; no prices or ids contain it as a value).
fn busy_spec() -> QuerySpec {
    QuerySpec::default()
        .with_filter(
            1,
            Filter::exact(vec!["cat".into(), "dog".into()], false, false),
        )
        .with_filter(2, Filter::range(Some("2.0".into()), None, false))
}

#[test]
fn distinct_ignores_only_its_own_column() {
    let (path, doc) = sample("distinct");
    let spec = busy_spec();

    // label: only the price filter applies -> rows a, c, e, f
    assert_eq!(labels_of(&doc, 1, Some(&spec)), [
        ("<null>".into(), 1),
        ("bird".into(), 1),
        ("cat".into(), 1),
        ("dog".into(), 1),
    ]);
    // price: only the label filter applies -> rows a, b, d, f
    let prices: Vec<String> = labels_of(&doc, 2, Some(&spec))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(prices, ["<null>", "1.0", "3.5", "7.0"]);
    // name: both filters apply -> rows a, f
    let names: Vec<String> = labels_of(&doc, 0, Some(&spec))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(names, ["a", "f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn distinct_and_histogram_apply_column_filters() {
    let (path, doc) = sample("search");
    // "cat" matches labels of rows a and d.
    let spec = QuerySpec::single(1, Filter::exact(vec!["cat".into()], false, false));
    let names: Vec<String> = labels_of(&doc, 0, Some(&spec))
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(names, ["a", "d"]);

    let h = doc.histogram(3, Some(&spec), 6).unwrap();
    assert_eq!(h.total, 2);
    assert_eq!(h.counts.iter().sum::<u64>(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_ignores_only_its_own_column() {
    let (path, doc) = sample("hist");
    let spec = busy_spec().with_filter(3, Filter::range(Some("60".into()), None, false));
    // id: label and price filters apply -> rows a, f; its own range does not.
    let h = doc.histogram(3, Some(&spec), 6).unwrap();
    assert_eq!(h.total, 2);
    // price: label and id filters apply -> row f only.
    let h = doc.histogram(2, Some(&spec), 4).unwrap();
    assert_eq!(h.total, 1);
    // Axis always spans the whole column.
    assert_eq!(h.lo, Some(1.0));
    assert_eq!(h.hi, Some(9.0));
    std::fs::remove_file(path).ok();
}

#[test]
fn view_histogram_and_summary_follow_the_current_view() {
    let (path, doc) = sample("view");
    let spec = busy_spec();
    let v = doc.apply_view(&spec).unwrap();
    assert_eq!(v.row_count, 2); // a, f

    let h = doc.view_histogram(v.version, 3, 6).unwrap();
    assert_eq!(h.total, 2);
    let s = doc.column_summary(2, 5).unwrap();
    assert_eq!(s.rows, 2);
    assert_eq!(s.min, Some(3.5));
    assert_eq!(s.max, Some(7.0));

    doc.apply_view(&QuerySpec::default()).unwrap();
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
    let bad = QuerySpec::single(0, Filter::range(Some("1".into()), None, false));
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

    let spec = QuerySpec::single(0, Filter::exact(vec!["12345".into()], false, false))
        .with_sort(0, true, 0);
    let worker = {
        let doc = doc.clone();
        std::thread::spawn(move || doc.apply_view(&spec))
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

#[test]
fn columns_with_too_many_values_are_counted_not_indexed() {
    // 150,000 distinct names, with "x" also on every 1000th row.
    let n = 150_000i64;
    let names: ArrayRef = Arc::new(StringArray::from_iter_values((0..n).map(|i| {
        if i % 1000 == 0 {
            "x".to_string()
        } else {
            format!("n{i:06}")
        }
    })));
    let batch = RecordBatch::try_from_iter(vec![("name", names)]).unwrap();
    let (path, doc) = write("many", &batch);

    let d = doc.distinct_values(0, None, 3).unwrap();
    assert!(!d.keyed);
    assert!(d.truncated);
    assert_eq!(d.total, (n - n / 1000 + 1) as u64);
    // The most common value comes first by count, but the list is in value
    // order: "x" sorts after every "n…".
    assert_eq!(d.values.len(), 3);
    assert_eq!(d.values[2].label.as_deref(), Some("x"));
    assert_eq!(d.values[2].count, (n / 1000) as u64);

    let by_value = QuerySpec::single(0, Filter::exact(vec!["x".into()], false, false));
    assert_eq!(
        doc.apply_view(&by_value).unwrap().row_count,
        (n / 1000) as u64
    );
    doc.apply_view(&QuerySpec::all(1)).unwrap();

    let s = doc.column_summary(0, 2).unwrap();
    assert_eq!(s.rows, n as u64);
    assert_eq!(s.null_count, 0);
    assert_eq!(s.distinct_count, d.total);
    assert_eq!(s.top_values[0].label.as_deref(), Some("x"));
    assert_eq!(s.min_label.as_deref(), Some("n000001"));
    assert_eq!(s.max_label.as_deref(), Some("x"));
    std::fs::remove_file(path).ok();
}

#[test]
fn summary_of_text_counts_nulls_apart() {
    let (path, doc) = sample("summary");
    let s = doc.column_summary(1, 2).unwrap();
    assert_eq!(s.null_count, 1);
    assert_eq!(s.distinct_count, 3);
    let top: Vec<_> = s
        .top_values
        .iter()
        .map(|v| (v.label.clone().unwrap(), v.count))
        .collect();
    // Ties stay in value order.
    assert_eq!(top, [("cat".to_string(), 2), ("dog".to_string(), 2)]);
    assert_eq!(s.min_label.as_deref(), Some("bird"));
    assert_eq!(s.max_label.as_deref(), Some("dog"));
    std::fs::remove_file(path).ok();
}

#[test]
fn summary_extrema_cover_single_values_nulls_and_filtered_views() {
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "text",
            Arc::new(StringArray::from(vec![
                Some("cat"),
                Some("dog"),
                Some("cat"),
                None,
            ])),
        ),
        (
            "boolean",
            Arc::new(BooleanArray::from(vec![
                Some(true),
                Some(true),
                Some(true),
                None,
            ])),
        ),
        ("null", Arc::new(StringArray::from(vec![None::<&str>; 4]))),
    ];
    let (path, doc) = write(
        "summary-extrema",
        &RecordBatch::try_from_iter(columns).unwrap(),
    );
    // A source singleton, a filtered singleton, null-only and empty views all
    // exercise the same summary contract, without separate files for each case.
    for (filter, expected) in [
        (None, [Some(("cat", "dog")), Some(("true", "true")), None]),
        (Some(Filter::exact(vec!["cat".into()], false, false)), [
            Some(("cat", "cat")),
            Some(("true", "true")),
            None,
        ]),
        (Some(Filter::exact(vec![], false, true)), [None; 3]),
        (Some(Filter::exact(vec![], false, false)), [None; 3]),
    ] {
        let mut spec = QuerySpec::default();
        if let Some(f) = filter {
            spec = spec.with_filter(0, f);
        }
        doc.apply_view(&spec).unwrap();
        for (column, extremes) in expected.into_iter().enumerate() {
            let summary = doc.column_summary(column as u32, 2).unwrap();
            assert_eq!(
                (summary.min_label.as_deref(), summary.max_label.as_deref()),
                (extremes.map(|(min, _)| min), extremes.map(|(_, max)| max)),
                "column {column}, view {:?}",
                doc.current_view()
            );
        }
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn min_and_max_leave_out_nan() {
    let v: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(2.0),
        Some(f64::NAN),
        None,
        Some(-1.5),
    ]));
    let batch = RecordBatch::try_from_iter(vec![("v", v)]).unwrap();
    let (path, doc) = write("nan", &batch);
    let s = doc.column_summary(0, 5).unwrap();
    assert_eq!(s.nan_count, 1);
    assert_eq!(s.min_label.as_deref(), Some("-1.5"));
    assert_eq!(s.max_label.as_deref(), Some("2.0"));
    std::fs::remove_file(path).ok();
}
