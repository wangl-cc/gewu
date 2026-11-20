#![cfg(feature = "parquet")]

//! Candidates and distributions are scoped to immutable views.

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{ColumnFilter, ColumnId, Filter, FilteredView};
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

fn labels_of(doc: &Arc<FilteredView>, column: ColumnId) -> Vec<(String, u64)> {
    let choices = doc.filter_choices(column).unwrap();
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

/// label in {cat, dog}, price >= 2.
fn busy_filters() -> Vec<ColumnFilter> {
    vec![
        ColumnFilter {
            column: ColumnId::from_raw(1),
            filter: Filter::exact(vec!["cat".into(), "dog".into()], false, false),
        },
        ColumnFilter {
            column: ColumnId::from_raw(2),
            filter: Filter::range(Some("2.0".into()), None, false),
        },
    ]
}

#[test]
fn choices_ignore_only_the_edited_columns_filter() {
    let (path, doc) = sample("distinct");
    let (c0, c1, c2) = (
        ColumnId::from_raw(0),
        ColumnId::from_raw(1),
        ColumnId::from_raw(2),
    );
    let view = doc.filter(&busy_filters()).unwrap();

    // An unfiltered column lists values of rows passing every filter: rows a and f.
    let names: Vec<String> = labels_of(&view, c0).into_iter().map(|(l, _)| l).collect();
    assert_eq!(names, ["a", "f"]);
    // A filtered column lists values of rows passing every other filter.
    for column in [c1, c2] {
        let others: Vec<_> = busy_filters()
            .into_iter()
            .filter(|filter| filter.column != column)
            .collect();
        let expected = labels_of(&doc.filter(&others).unwrap(), column);
        assert_eq!(labels_of(&view, column), expected);
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn choices_and_distribution_apply_column_filters() {
    let (path, doc) = sample("search");
    let (c0, c1, c3) = (
        ColumnId::from_raw(0),
        ColumnId::from_raw(1),
        ColumnId::from_raw(3),
    );
    // "cat" matches labels of rows a and d.
    let view = doc
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::exact(vec!["cat".into()], false, false),
        }])
        .unwrap();
    let names: Vec<String> = labels_of(&view, c0).into_iter().map(|(l, _)| l).collect();
    assert_eq!(names, ["a", "d"]);

    let h = view
        .filter_distribution(c3, 6, gewu_core::AxisScale::Linear)
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!(h.counts.iter().sum::<u64>(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_domain_and_counts_use_rows_passing_the_other_filters() {
    let (path, doc) = sample("hist");
    let (c2, c3) = (ColumnId::from_raw(2), ColumnId::from_raw(3));
    let mut filters = busy_filters();
    filters.push(ColumnFilter {
        column: c3,
        filter: Filter::range(Some("60".into()), None, false),
    });
    let view = doc.filter(&filters).unwrap();
    for column in [c2, c3] {
        let others: Vec<_> = filters
            .iter()
            .filter(|filter| filter.column != column)
            .cloned()
            .collect();
        let histogram = |view: &std::sync::Arc<gewu_core::FilteredView>| {
            view.filter_distribution(column, 4, gewu_core::AxisScale::Linear)
                .map(|result| result.expect("histogram domain").histogram)
                .unwrap()
        };
        let expected = histogram(&doc.filter(&others).unwrap());
        let actual = histogram(&view);
        assert_eq!(actual.counts, expected.counts);
        assert_eq!(actual.min_label, expected.min_label);
        assert_eq!(actual.max_label, expected.max_label);
    }
    std::fs::remove_file(path).ok();
}
