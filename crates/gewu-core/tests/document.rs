use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Bound, Document, Error, Filter, SortKey, ViewState};
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

fn range(column: u32, min: Option<Bound>, max: Option<Bound>) -> ViewState {
    ViewState {
        filters: vec![Filter::Range { column, min, max }],
        ..Default::default()
    }
}

#[test]
fn range_filter_on_floats_skips_nulls() {
    let (path, doc) = sample();
    // price: a 3.5, b 1.0, c 9.0, d null, e 2.25, f 7.0
    doc.apply_view(&range(2, Some(Bound::Float(2.25)), Some(Bound::Int(7))))
        .unwrap();
    assert_eq!(names(&doc), ["a", "e", "f"]);

    // Open-ended bounds.
    doc.apply_view(&range(2, Some(Bound::Float(7.0)), None))
        .unwrap();
    assert_eq!(names(&doc), ["c", "f"]);
    doc.apply_view(&range(2, None, Some(Bound::Float(1.0))))
        .unwrap();
    assert_eq!(names(&doc), ["b"]);
    // No bounds keeps every non-null value.
    doc.apply_view(&range(2, None, None)).unwrap();
    assert_eq!(names(&doc), ["a", "b", "c", "e", "f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_on_integers() {
    let (path, doc) = sample();
    // id: 10, 20, 30, 40, 50, 60. Float bounds round inward.
    doc.apply_view(&range(
        3,
        Some(Bound::Float(19.5)),
        Some(Bound::Float(40.9)),
    ))
    .unwrap();
    assert_eq!(names(&doc), ["b", "c", "d"]);
    doc.apply_view(&range(3, Some(Bound::UInt(u64::MAX)), None))
        .unwrap();
    assert_eq!(names(&doc), Vec::<String>::new());
    doc.apply_view(&range(3, Some(Bound::Int(60)), Some(Bound::Int(60))))
        .unwrap();
    assert_eq!(names(&doc), ["f"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_rejects_text_columns() {
    let (path, doc) = sample();
    assert!(matches!(
        doc.apply_view(&range(1, Some(Bound::Int(0)), None)),
        Err(Error::InvalidArgument(_))
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_counts() {
    let (path, doc) = sample();
    // price spans 1.0..=9.0 in 4 bins of width 2: [1,3) [3,5) [5,7) [7,9].
    let h = doc.histogram(2, None, 4).unwrap();
    assert_eq!((h.lo, h.hi), (Some(1.0), Some(9.0)));
    assert!(!h.integer);
    assert_eq!(h.counts, [2, 1, 0, 2]);
    assert_eq!((h.null_count, h.total), (1, 6));

    // Small integer span: one bin per integer would need 51 bins, so with
    // 100 bins each integer 10..=60 gets its own.
    let h = doc.histogram(3, None, 100).unwrap();
    assert!(h.integer);
    assert_eq!((h.lo, h.hi), (Some(10.0), Some(61.0)));
    assert_eq!(h.counts.len(), 51);
    assert_eq!(h.counts.iter().sum::<u64>(), 6);
    assert_eq!(h.counts[0], 1);
    assert_eq!(h.counts[50], 1);

    // Larger integer span than bins: still ends at max + 1, so the last
    // bin holds the max and a range up to it is exact.
    let h = doc.histogram(3, None, 5).unwrap();
    assert_eq!((h.lo, h.hi), (Some(10.0), Some(61.0)));
    assert_eq!(h.counts.len(), 5);
    assert_eq!(h.counts.iter().sum::<u64>(), 6);
    assert_eq!(h.counts[4], 1);

    // Counts follow filters on other columns; the axis does not move, and
    // the column's own range filter is ignored.
    let keys = doc.keys_for_labels(1, &[Some("cat".into())]).unwrap();
    let state = ViewState {
        filters: vec![
            Filter::InValues { column: 1, keys },
            Filter::Range {
                column: 2,
                min: Some(Bound::Float(100.0)),
                max: None,
            },
        ],
        ..Default::default()
    };
    let h = doc.histogram(2, Some(&state), 4).unwrap();
    assert_eq!((h.lo, h.hi), (Some(1.0), Some(9.0)));
    assert_eq!(h.counts, [0, 1, 0, 0]);
    assert_eq!((h.null_count, h.total), (1, 2));

    assert!(doc.histogram(1, None, 10).is_err());
    std::fs::remove_file(path).ok();
}
