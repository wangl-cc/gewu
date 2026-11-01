#![cfg(feature = "parquet")]

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{Document, Error, Filter, QuerySpec, SortKey};
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
    let spec = QuerySpec {
        sort: vec![SortKey {
            column: 2,
            descending: false,
        }],
        ..Default::default()
    };
    doc.apply_view(&spec).unwrap();
    assert_eq!(names(&doc), ["b", "e", "a", "f", "c", "d"]);

    let spec = QuerySpec {
        sort: vec![SortKey {
            column: 2,
            descending: true,
        }],
        ..Default::default()
    };
    doc.apply_view(&spec).unwrap();
    assert_eq!(names(&doc), ["c", "f", "a", "e", "b", "d"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn checkbox_filter_and_distinct() {
    let (path, doc) = sample();
    let all = doc.distinct_values(1, None, 100).unwrap();
    let labels: Vec<_> = all.values.iter().map(|v| v.label.clone()).collect();
    assert_eq!(labels, [
        None,
        Some("bird".into()),
        Some("cat".into()),
        Some("dog".into())
    ]);
    assert_eq!(all.values[2].count, 2);

    let keys = doc
        .keys_for_labels(1, &[Some("cat".into()), Some("dog".into())])
        .unwrap();
    let spec = QuerySpec::single(1, Filter::InValues { keys }).with_sort(vec![SortKey {
        column: 3,
        descending: true,
    }]);
    let v = doc.apply_view(&spec).unwrap();
    assert_eq!(v.row_count, 4);
    assert_eq!(names(&doc), ["f", "d", "b", "a"]);

    // Distinct values of another column respect this filter.
    let prices = doc.distinct_values(2, Some(&spec), 100).unwrap();
    assert_eq!(prices.total, 4);
    // ...but the filtered column itself still lists every value (like Excel).
    let labels = doc.distinct_values(1, Some(&spec), 100).unwrap();
    assert_eq!(labels.total, 4);
    std::fs::remove_file(path).ok();
}

#[test]
fn null_contains_and_typed_equality() {
    let (path, doc) = sample();
    let spec = QuerySpec::single(2, Filter::IsNull { negate: false });
    doc.apply_view(&spec).unwrap();
    assert_eq!(names(&doc), ["d"]);

    let spec = QuerySpec::single(1, Filter::Contains {
        include_nulls: false,
        text: "D".into(),
        case_sensitive: false,
    });
    doc.apply_view(&spec).unwrap();
    assert_eq!(names(&doc), ["b", "e", "f"]);

    let spec = QuerySpec::single(2, Filter::Equals {
        value: "2.25".into(),
    });
    doc.apply_view(&spec).unwrap();
    assert_eq!(names(&doc), ["e"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn stale_version_is_rejected() {
    let (path, doc) = sample();
    let old = doc.current_view().version;
    doc.apply_view(&QuerySpec::default()).unwrap();
    assert!(matches!(
        doc.rows(old, 0, 10, &[0]),
        Err(Error::StaleVersion { .. })
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn stats() {
    let (path, doc) = sample();
    let s = doc.column_summary(2, 5).unwrap();
    assert_eq!(s.null_count, 1);
    assert_eq!(s.distinct_count, 5);
    assert_eq!(s.min_label.as_deref(), Some("1.0"));
    assert_eq!(s.max_label.as_deref(), Some("9.0"));
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_on_floats_skips_nulls() {
    let (path, doc) = sample();
    // price: a 3.5, b 1.0, c 9.0, d null, e 2.25, f 7.0.
    // Include closed, open-ended, and unbounded ranges (nulls excluded).
    for (min, max, expected) in [
        (Some("2.25"), Some("7"), &["a", "e", "f"][..]),
        (Some("7.0"), None, &["c", "f"]),
        (None, Some("1.0"), &["b"]),
        (None, None, &["a", "b", "c", "e", "f"]),
    ] {
        let min = min.map(|text| doc.parse_bound(2, text).unwrap());
        let max = max.map(|text| doc.parse_bound(2, text).unwrap());
        doc.apply_view(&QuerySpec::single(2, Filter::Range {
            include_nulls: false,
            min,
            max,
        }))
        .unwrap();
        assert_eq!(names(&doc), expected);
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_on_integers() {
    let (path, doc) = sample();
    // id: 10, 20, 30, 40, 50, 60. Fractional bounds round inward.
    for (min, max, expected) in [
        (Some("19.5"), Some("40.9"), &["b", "c", "d"][..]),
        (Some("18446744073709551615"), None, &[]),
        (Some("60"), Some("60"), &["f"]),
    ] {
        let min = min.map(|text| doc.parse_bound(3, text).unwrap());
        let max = max.map(|text| doc.parse_bound(3, text).unwrap());
        doc.apply_view(&QuerySpec::single(3, Filter::Range {
            include_nulls: false,
            min,
            max,
        }))
        .unwrap();
        assert_eq!(names(&doc), expected);
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_rejects_text_columns() {
    let (path, doc) = sample();
    assert!(matches!(
        doc.parse_bound(1, "0"),
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

    assert!(doc.histogram(1, None, 10).is_err());
    std::fs::remove_file(path).ok();
}

/// name | day (date32) | at (timestamp µs, UTC) | n (int64) | x (float64)
fn temporal_sample() -> (PathBuf, Document) {
    use arrow::array::{Date32Array, TimestampMicrosecondArray};
    let names: ArrayRef = Arc::new(StringArray::from(vec!["Alpha", "beta", "Gamma", "delta"]));
    // 2024-01-01, 2024-01-02, null, 2024-03-15
    let days: ArrayRef = Arc::new(Date32Array::from(vec![
        Some(19_723),
        Some(19_724),
        None,
        Some(19_797),
    ]));
    // 2024-01-01T09:00:00Z, +1h, +2h, +3h
    let base = 1_704_099_600_000_000i64;
    let at: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from(vec![
            base,
            base + 3_600_000_000,
            base + 7_200_000_000,
            base + 10_800_000_000,
        ])
        .with_timezone("UTC"),
    );
    let n: ArrayRef = Arc::new(Int64Array::from(vec![7, 170, 1700, -7]));
    let x: ArrayRef = Arc::new(Float64Array::from(vec![0.5, 2.25, 17.0, f64::NAN]));
    let batch = RecordBatch::try_from_iter(vec![
        ("name", names),
        ("day", days),
        ("at", at),
        ("n", n),
        ("x", x),
    ])
    .unwrap();
    let path = std::env::temp_dir().join(format!(
        "gewu-temporal-{}-{:?}.parquet",
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

fn find_names(doc: &Document, text: &str) -> Vec<String> {
    let names = names(doc);
    let hits = doc
        .prepare_find(doc.current_view().version, text, &[0, 1, 2, 3, 4])
        .unwrap()
        .next(None, 256)
        .unwrap();
    let mut rows: Vec<usize> = hits.iter().map(|hit| hit.display_row as usize).collect();
    rows.dedup();
    rows.into_iter().map(|row| names[row].clone()).collect()
}

// n: 7, 170, 1700, -7 · x: 0.5, 2.25, 17.0, NaN
// day: 2024-01-01, 2024-01-02, null, 2024-03-15 · at: 2024-01-01 09:00..12:00 UTC

#[test]
fn search_text_is_case_insensitive_substring() {
    let (path, doc) = temporal_sample();
    assert_eq!(find_names(&doc, "ALPHA"), ["Alpha"]);
    assert_eq!(find_names(&doc, "gam"), ["Gamma"]);
    // Characters that are special in LIKE patterns match literally.
    assert_eq!(find_names(&doc, "%"), Vec::<String>::new());
    assert_eq!(find_names(&doc, "_"), Vec::<String>::new());
    std::fs::remove_file(path).ok();
}

#[test]
fn search_integers_by_value() {
    let (path, doc) = temporal_sample();
    // Exact value, not a substring: 170 does not match 1700.
    assert_eq!(find_names(&doc, "170"), ["beta"]);
    assert_eq!(find_names(&doc, "-7"), ["delta"]);
    assert_eq!(find_names(&doc, "+7"), ["Alpha"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn search_floats_by_typed_precision() {
    let (path, doc) = temporal_sample();
    assert_eq!(find_names(&doc, "2.25"), ["beta"]);
    // "2.3" means [2.25, 2.35): includes 2.25.
    assert_eq!(find_names(&doc, "2.3"), ["beta"]);
    assert_eq!(find_names(&doc, "2.2"), Vec::<String>::new());
    // "17" means [16.5, 17.5) for floats; ints need exactly 17.
    assert_eq!(find_names(&doc, "17"), ["Gamma"]);
    assert_eq!(find_names(&doc, "5e-1"), ["Alpha"]);
    assert_eq!(find_names(&doc, "NaN"), ["delta"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn search_dates_by_period() {
    let (path, doc) = temporal_sample();
    // A month or day matches the whole period; null dates never match.
    assert_eq!(find_names(&doc, "2024-03"), ["delta"]);
    assert_eq!(find_names(&doc, "2024-01-02"), ["beta"]);
    // Every timestamp is in 2024, so the year matches all rows.
    assert_eq!(find_names(&doc, "2024"), [
        "Alpha", "beta", "Gamma", "delta"
    ]);
    // Hours and minutes apply to timestamps only.
    assert_eq!(find_names(&doc, "2024-01-01 10"), ["beta"]);
    assert_eq!(find_names(&doc, "2024-01-01T11:00"), ["Gamma"]);
    assert_eq!(find_names(&doc, "2024-01-01 11:01"), Vec::<String>::new());
    // Not a date: skipped, not an error.
    assert_eq!(find_names(&doc, "2024-13"), Vec::<String>::new());
    std::fs::remove_file(path).ok();
}

#[test]
fn contains_filter_is_for_text_only() {
    let (path, doc) = temporal_sample();
    let before = doc.current_view();
    // Numbers and dates are filtered by range; "contains" would mean
    // something different for them, so it is refused.
    for column in [1, 3] {
        let err = doc
            .apply_view(&QuerySpec::single(column, Filter::Contains {
                include_nulls: false,
                text: "1700".into(),
                case_sensitive: false,
            }))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
    }
    assert_eq!(doc.current_view(), before);
    std::fs::remove_file(path).ok();
}

#[test]
fn dates_format_parse_and_filter() {
    let (path, doc) = temporal_sample();
    assert_eq!(doc.format_value(1, 19_724).unwrap(), "2024-01-02");
    assert_eq!(
        doc.parse_bound(1, "2024-01-02").unwrap().axis_value(),
        19_724.0
    );
    assert!(doc.parse_bound(1, "not a date").is_err());

    let base = 1_704_099_600_000_000i64;
    assert_eq!(doc.format_value(2, base).unwrap(), "2024-01-01T09:00:00Z");
    assert_eq!(
        doc.parse_bound(2, "2024-01-01T10:00:00Z")
            .unwrap()
            .axis_value(),
        (base + 3_600_000_000) as f64
    );

    // Range on dates: nulls excluded, bounds inclusive.
    let min = doc.parse_bound(1, "2024-01-02").unwrap();
    doc.apply_view(&QuerySpec::single(1, Filter::Range {
        include_nulls: false,
        min: Some(min),
        max: None,
    }))
    .unwrap();
    assert_eq!(names(&doc), ["beta", "delta"]);

    // Range on timestamps.
    let max = doc.parse_bound(2, "2024-01-01T10:00:00Z").unwrap();
    doc.apply_view(&QuerySpec::single(2, Filter::Range {
        include_nulls: false,
        min: None,
        max: Some(max),
    }))
    .unwrap();
    assert_eq!(names(&doc), ["Alpha", "beta"]);

    // Dates are integers: the histogram ends one day past the last date.
    let h = doc.histogram(1, None, 100).unwrap();
    assert!(h.integer);
    assert_eq!((h.lo, h.hi), (Some(19_723.0), Some(19_798.0)));
    assert_eq!(h.null_count, 1);
    std::fs::remove_file(path).ok();
}

#[test]
fn find_marks_matching_cells() {
    let (path, doc) = temporal_sample();
    // "170" matches n = 170 only; "2024-01-02" matches the day column.
    for (query, expected) in [("170", vec![(1, 3)]), ("2024-01-02", vec![(1, 1)])] {
        let find = doc.prepare_find(0, query, &[0, 1, 2, 3, 4]).unwrap();
        let hits: Vec<(u64, u32)> = find
            .next(None, 10)
            .unwrap()
            .into_iter()
            .map(|hit| (hit.display_row, hit.column))
            .collect();
        assert_eq!(hits, expected, "query {query}");
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn column_summary_follows_the_view() {
    let (path, doc) = sample();
    // price: 3.5, 1.0, 9.0, null, 2.25, 7.0
    let s = doc.column_summary(2, 5).unwrap();
    assert_eq!((s.rows, s.null_count, s.distinct_count), (6, 1, 5));
    assert_eq!((s.min, s.max), (Some(1.0), Some(9.0)));
    assert!((s.mean.unwrap() - 4.55).abs() < 1e-9);
    // Sample standard deviation: sqrt(44.8 / 4).
    assert!((s.std_dev.unwrap() - 3.3466).abs() < 1e-3);
    assert!(s.top_values.is_empty());

    // label: cat, dog, null, cat, bird, dog
    let s = doc.column_summary(1, 2).unwrap();
    assert_eq!((s.null_count, s.distinct_count), (1, 3));
    let top: Vec<_> = s
        .top_values
        .iter()
        .map(|v| (v.label.clone().unwrap(), v.count))
        .collect();
    assert_eq!(top, [("cat".to_string(), 2), ("dog".to_string(), 2)]);

    // With a filter, only the view's rows count.
    let keys = doc.keys_for_labels(1, &[Some("dog".into())]).unwrap();
    doc.apply_view(&QuerySpec::single(1, Filter::InValues { keys }))
        .unwrap();
    let s = doc.column_summary(2, 5).unwrap();
    assert_eq!((s.rows, s.min, s.max), (2, Some(1.0), Some(7.0)));
    std::fs::remove_file(path).ok();
}
