#![cfg(feature = "parquet")]

//! Candidates and distributions are scoped to immutable views.

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Cancellation, Error, Filter, QuerySpec};
use parquet::arrow::ArrowWriter;

/// name | label | price | id
fn sample(tag: &str) -> (PathBuf, Arc<gewu_core::Dataset>) {
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

fn write(tag: &str, batch: &RecordBatch) -> (PathBuf, Arc<gewu_core::Dataset>) {
    let path = std::env::temp_dir().join(format!(
        "gewu-facets-{tag}-{}-{:?}.parquet",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(batch).unwrap();
    w.close().unwrap();
    let doc = Arc::new(gewu_core::Dataset::open(&path).unwrap());
    (path, doc)
}

fn labels_of(doc: &std::sync::Arc<gewu_core::View>, column: u32) -> Vec<(String, u64)> {
    let choices = doc
        .filter_choices(column, &Cancellation::default())
        .unwrap();
    let mut result = Vec::new();
    if choices.null_count > 0 {
        result.push(("<null>".into(), choices.null_count));
    }
    result.extend(
        choices
            .values
            .unwrap()
            .into_iter()
            .filter(|v| v.count > 0)
            .map(|v| (v.label, v.count)),
    );
    result
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
fn choices_include_only_visible_values() {
    let (path, doc) = sample("distinct");
    let spec = busy_spec();
    let view = doc.query(&spec, &Cancellation::default()).unwrap();

    // Both filters apply to all candidate columns: rows a and f.
    assert_eq!(labels_of(&view, 1), [("cat".into(), 1), ("dog".into(), 1),]);
    let prices: Vec<String> = labels_of(&view, 2).into_iter().map(|(l, _)| l).collect();
    assert_eq!(prices, ["3.5", "7.0"]);
    // name: both filters apply -> rows a, f
    let names: Vec<String> = labels_of(&view, 0).into_iter().map(|(l, _)| l).collect();
    assert_eq!(names, ["a", "f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn choices_and_distribution_apply_column_filters() {
    let (path, doc) = sample("search");
    // "cat" matches labels of rows a and d.
    let spec = QuerySpec::single(1, Filter::exact(vec!["cat".into()], false, false));
    let view = doc.query(&spec, &Cancellation::default()).unwrap();
    let names: Vec<String> = labels_of(&view, 0).into_iter().map(|(l, _)| l).collect();
    assert_eq!(names, ["a", "d"]);

    let h = doc
        .query(&spec, &gewu_core::Cancellation::default())
        .unwrap()
        .filter_distribution(3, 6, gewu_core::AxisScale::Linear, &Cancellation::default())
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!(h.counts.iter().sum::<u64>(), 2);
    assert_eq!(h.counts.iter().sum::<u64>(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_domain_and_counts_use_visible_rows() {
    let (path, doc) = sample("hist");
    let spec = busy_spec().with_filter(3, Filter::range(Some("60".into()), None, false));
    // All filters apply: only row f remains.
    let h = doc
        .query(&spec, &gewu_core::Cancellation::default())
        .unwrap()
        .filter_distribution(3, 6, gewu_core::AxisScale::Linear, &Cancellation::default())
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!(h.counts.iter().sum::<u64>(), 1);
    // The price domain contains only the visible value.
    let h = doc
        .query(&spec, &gewu_core::Cancellation::default())
        .unwrap()
        .filter_distribution(2, 4, gewu_core::AxisScale::Linear, &Cancellation::default())
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!(h.counts.iter().sum::<u64>(), 1);
    // A constant visible domain does not fall back to the source range.
    assert_eq!(h.min_label.as_deref(), Some("7"));
    assert_eq!(h.max_label.as_deref(), Some("7"));
    assert!(h.lo.unwrap() <= 7.0 && h.hi.unwrap() >= 7.0);
    std::fs::remove_file(path).ok();
}

#[test]
fn failed_apply_keeps_the_old_view() {
    let (path, doc) = sample("failed");
    let before = doc.view();
    let bad = QuerySpec::single(0, Filter::range(Some("1".into()), None, false));
    assert!(doc
        .query(&bad, &gewu_core::Cancellation::default())
        .is_err());
    assert_eq!(
        doc.view().rows(0, 6, &[0]).unwrap().cells,
        before.rows(0, 6, &[0]).unwrap().cells
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn unknown_column_is_an_argument_error() {
    let (path, doc) = sample("unknown");
    assert!(matches!(
        doc.view().filter_choices(9, &Cancellation::default()),
        Err(Error::InvalidArgument(_))
    ));
    assert!(matches!(
        doc.view()
            .filter_distribution(
                9,
                10,
                gewu_core::AxisScale::Linear,
                &Cancellation::default()
            )
            .map(|result| result.expect("histogram domain").histogram),
        Err(Error::InvalidArgument(_))
    ));
    let v = doc.view();
    assert!(matches!(v.rows(0, 1, &[9]), Err(Error::InvalidArgument(_))));
    std::fs::remove_file(path).ok();
}

#[test]
fn cancellation_belongs_to_one_request() {
    let (path, doc) = sample("cancel");
    let token = Cancellation::default();
    token.cancel();
    let spec = busy_spec();
    assert!(matches!(doc.query(&spec, &token), Err(Error::Cancelled)));
    let view = doc.query(&spec, &Cancellation::default()).unwrap();
    assert_eq!(view.row_count(), 2);
    assert_eq!(doc.view().row_count(), 6);
    std::fs::remove_file(path).ok();
}

#[test]
fn high_cardinality_columns_have_no_choice_list_but_exact_filters_still_work() {
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

    let choices = doc
        .view()
        .filter_choices(0, &Cancellation::default())
        .unwrap();
    assert!(choices.values.is_none());
    assert_eq!(choices.null_count, 0);
    let by_value = QuerySpec::single(0, Filter::exact(vec!["x".into()], false, false));
    assert_eq!(
        doc.query(&by_value, &gewu_core::Cancellation::default())
            .unwrap()
            .row_count(),
        (n / 1000) as u64
    );
    doc.query(&QuerySpec::all(1), &gewu_core::Cancellation::default())
        .unwrap();

    std::fs::remove_file(path).ok();
}

#[test]
fn value_analysis_excludes_nulls_and_recounts_candidates() {
    let (path, doc) = sample("summary");
    let summary = doc
        .view()
        .analyze_values(1, &gewu_core::Cancellation::default())
        .unwrap();
    assert_eq!(summary.distinct_count, 3);
    let top: Vec<_> = summary
        .values
        .iter()
        .map(|v| (v.label.as_str(), v.count))
        .collect();
    assert_eq!(top, [("cat", 2), ("dog", 2), ("bird", 1)]);
    assert_eq!(
        doc.view().count_nulls(1, &Cancellation::default()).unwrap(),
        1
    );
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
    let s = doc
        .view()
        .analyze_statistics(0, &gewu_core::Cancellation::default())
        .unwrap();
    assert_eq!(s.nan_count, 1);
    assert_eq!(s.min_label.as_deref(), Some("-1.5"));
    assert_eq!(s.max_label.as_deref(), Some("2.0"));
    std::fs::remove_file(path).ok();
}
