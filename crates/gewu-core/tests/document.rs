#![cfg(feature = "parquet")]

use std::{path::PathBuf, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::{ColumnFilter, ColumnId, ColumnSort, Error, Filter, OrderedView};
use parquet::arrow::ArrowWriter;

/// name | label | price
fn sample() -> (PathBuf, Arc<gewu_core::Dataset>) {
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
    let doc = Arc::new(gewu_core::Dataset::open(&path).unwrap());
    (path, doc)
}

fn names(doc: &Arc<OrderedView>) -> Vec<String> {
    let block = doc.cell_text(0, 100, &[ColumnId::from_raw(0)]).unwrap();
    (0..block.row_count())
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
    assert_eq!(info.columns[1].id, ColumnId::from_raw(1));
    assert_eq!(doc.unfiltered().row_count(), 6);
    assert_eq!(names(&doc.unfiltered().unsorted()), [
        "a", "b", "c", "d", "e", "f"
    ]);
    std::fs::remove_file(path).ok();
}

#[test]
fn sorts_with_nulls_last() {
    let (path, dataset) = sample();
    let c2 = ColumnId::from_raw(2);
    let filtered = dataset.unfiltered();
    let asc = filtered
        .sort(&[ColumnSort {
            column: c2,
            descending: false,
        }])
        .unwrap();
    assert_eq!(names(&asc), ["b", "e", "a", "f", "c", "d"]);

    let desc = filtered
        .sort(&[ColumnSort {
            column: c2,
            descending: true,
        }])
        .unwrap();
    assert_eq!(names(&desc), ["c", "f", "a", "e", "b", "d"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn checkbox_filter_and_choices() {
    let (path, doc) = sample();
    let (c1, c2, c3) = (
        ColumnId::from_raw(1),
        ColumnId::from_raw(2),
        ColumnId::from_raw(3),
    );
    let all = doc.unfiltered().filter_choices(c1).unwrap();
    assert_eq!(all.null_count, 1);
    let values = all.values.unwrap();
    let labels: Vec<_> = values.iter().map(|v| v.label.as_str()).collect();
    assert_eq!(labels, ["bird", "cat", "dog"]);
    assert_eq!(values[1].count, 2);

    let filtered = doc
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::exact(vec!["cat".into(), "dog".into()], false, false),
        }])
        .unwrap();
    let v = filtered
        .sort(&[ColumnSort {
            column: c3,
            descending: true,
        }])
        .unwrap();
    assert_eq!(v.row_count(), 4);
    assert_eq!(names(&v), ["f", "d", "b", "a"]);

    // Distinct values of another column respect this filter.
    let prices = filtered.filter_choices(c2).unwrap();
    assert_eq!(prices.null_count, 1);
    assert_eq!(
        prices
            .values
            .unwrap()
            .iter()
            .filter(|v| v.count > 0)
            .count(),
        3
    );
    // The filtered column also lists only values present in this view.
    let labels = filtered.filter_choices(c1).unwrap();
    assert_eq!(labels.null_count, 0);
    assert_eq!(labels.values.unwrap().len(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn null_contains_and_typed_equality() {
    let (path, dataset) = sample();
    let (c1, c2) = (ColumnId::from_raw(1), ColumnId::from_raw(2));
    let doc = dataset
        .filter(&[ColumnFilter {
            column: c2,
            filter: Filter::exact(vec![], false, true),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(names(&doc), ["d"]);

    let doc = dataset
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::contains("D".into(), false, false),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(names(&doc), ["b", "e", "f"]);

    let doc = dataset
        .filter(&[ColumnFilter {
            column: c2,
            filter: Filter::exact(vec!["2.25".into()], false, false),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(names(&doc), ["e"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn dataset_queries_recompute_from_the_full_source() {
    let (path, dataset) = sample();
    let c1 = ColumnId::from_raw(1);
    let filtered = dataset
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::exact(vec!["cat".into()], false, false),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(names(&filtered), ["a", "d"]);
    let restored = dataset.filter(&[]).unwrap().unsorted();
    assert_eq!(names(&restored), names(&dataset.unfiltered().unsorted()));
    assert_eq!(names(&filtered), ["a", "d"]);
    drop(dataset);
    assert_eq!(filtered.row_count(), 2);
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_on_floats_skips_nulls() {
    let (path, doc) = sample();
    let c2 = ColumnId::from_raw(2);
    // price: a 3.5, b 1.0, c 9.0, d null, e 2.25, f 7.0.
    // Include closed, open-ended, and unbounded ranges (nulls excluded).
    for (min, max, expected) in [
        (Some("2.25"), Some("7"), &["a", "e", "f"][..]),
        (Some("7.0"), None, &["c", "f"]),
        (None, Some("1.0"), &["b"]),
        (None, None, &["a", "b", "c", "e", "f"]),
    ] {
        let min = min.map(|text| (text).to_owned());
        let max = max.map(|text| (text).to_owned());
        let view = doc
            .filter(&[ColumnFilter {
                column: c2,
                filter: Filter::range(min, max, false),
            }])
            .unwrap()
            .unsorted();
        assert_eq!(names(&view), expected);
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_on_integers() {
    let (path, doc) = sample();
    let c3 = ColumnId::from_raw(3);
    // id: 10, 20, 30, 40, 50, 60. Fractional bounds round inward.
    for (min, max, expected) in [
        (Some("19.5"), Some("40.9"), &["b", "c", "d"][..]),
        (Some("18446744073709551615"), None, &[]),
        (Some("60"), Some("60"), &["f"]),
    ] {
        let min = min.map(|text| (text).to_owned());
        let max = max.map(|text| (text).to_owned());
        let view = doc
            .filter(&[ColumnFilter {
                column: c3,
                filter: Filter::range(min, max, false),
            }])
            .unwrap()
            .unsorted();
        assert_eq!(names(&view), expected);
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn range_filter_rejects_text_columns() {
    let (path, doc) = sample();
    assert!(matches!(
        doc.filter(&[ColumnFilter {
            column: ColumnId::from_raw(1),
            filter: Filter::range(Some(("0").to_owned()), None, false),
        }]),
        Err(Error::Filter(_))
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn histogram_counts() {
    let (path, doc) = sample();
    let (c1, c2, c3) = (
        ColumnId::from_raw(1),
        ColumnId::from_raw(2),
        ColumnId::from_raw(3),
    );
    let view = doc.unfiltered();
    // price spans 1.0..=9.0 in 4 bins of width 2: [1,3) [3,5) [5,7) [7,9].
    let h = view
        .filter_distribution(c2, 4, gewu_core::AxisScale::Linear)
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert_eq!((h.lo, h.hi), (Some(1.0), Some(9.0)));
    assert!(!h.integer);
    assert_eq!(h.counts, [2, 1, 0, 2]);
    assert_eq!((view.count_nulls(c2).unwrap(), view.row_count()), (1, 6));

    // Small integer span: one bin per integer would need 51 bins, so with
    // 60 bins each integer 10..=60 gets its own.
    let h = view
        .filter_distribution(c3, 60, gewu_core::AxisScale::Linear)
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert!(h.integer);
    assert_eq!((h.lo, h.hi), (Some(10.0), Some(61.0)));
    assert_eq!(h.counts.len(), 51);
    assert_eq!(h.counts.iter().sum::<u64>(), 6);
    assert_eq!(h.counts[0], 1);
    assert_eq!(h.counts[50], 1);

    assert!(view
        .filter_distribution(c1, 10, gewu_core::AxisScale::Linear)
        .unwrap()
        .is_none());
    std::fs::remove_file(path).ok();
}

/// name | day (date32) | at (timestamp µs, UTC) | n (int64) | x (float64)
fn temporal_sample() -> (PathBuf, Arc<gewu_core::Dataset>) {
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
    let doc = Arc::new(gewu_core::Dataset::open(&path).unwrap());
    (path, doc)
}

fn find_names(doc: &Arc<OrderedView>, text: &str) -> Vec<String> {
    let names = names(doc);
    let cols: Vec<ColumnId> = (0..5).map(ColumnId::from_raw).collect();
    let hits = doc.find(text, &cols).unwrap().next(None, 256).unwrap();
    let mut rows: Vec<usize> = hits.iter().map(|hit| hit.display_row as usize).collect();
    rows.dedup();
    rows.into_iter().map(|row| names[row].clone()).collect()
}

// n: 7, 170, 1700, -7 · x: 0.5, 2.25, 17.0, NaN
// day: 2024-01-01, 2024-01-02, null, 2024-03-15 · at: 2024-01-01 09:00..12:00 UTC

#[test]
fn search_text_is_case_insensitive_substring() {
    let (path, doc) = temporal_sample();
    let ordered = doc.unfiltered().unsorted();
    assert_eq!(find_names(&ordered, "ALPHA"), ["Alpha"]);
    assert_eq!(find_names(&ordered, "gam"), ["Gamma"]);
    // Characters that are special in LIKE patterns match literally.
    assert_eq!(find_names(&ordered, "%"), Vec::<String>::new());
    assert_eq!(find_names(&ordered, "_"), Vec::<String>::new());
    std::fs::remove_file(path).ok();
}

#[test]
fn search_integers_by_value() {
    let (path, doc) = temporal_sample();
    let ordered = doc.unfiltered().unsorted();
    // Exact value, not a substring: 170 does not match 1700.
    assert_eq!(find_names(&ordered, "170"), ["beta"]);
    assert_eq!(find_names(&ordered, "-7"), ["delta"]);
    assert_eq!(find_names(&ordered, "+7"), ["Alpha"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn search_floats_by_typed_precision() {
    let (path, doc) = temporal_sample();
    let ordered = doc.unfiltered().unsorted();
    assert_eq!(find_names(&ordered, "2.25"), ["beta"]);
    // "2.3" means [2.25, 2.35): includes 2.25.
    assert_eq!(find_names(&ordered, "2.3"), ["beta"]);
    assert_eq!(find_names(&ordered, "2.2"), Vec::<String>::new());
    // "17" means [16.5, 17.5) for floats; ints need exactly 17.
    assert_eq!(find_names(&ordered, "17"), ["Gamma"]);
    assert_eq!(find_names(&ordered, "5e-1"), ["Alpha"]);
    assert_eq!(find_names(&ordered, "NaN"), ["delta"]);
    std::fs::remove_file(path).ok();
}

#[test]
fn search_dates_by_period() {
    let (path, doc) = temporal_sample();
    let ordered = doc.unfiltered().unsorted();
    // A month or day matches the whole period; null dates never match.
    assert_eq!(find_names(&ordered, "2024-03"), ["delta"]);
    assert_eq!(find_names(&ordered, "2024-01-02"), ["beta"]);
    // Every timestamp is in 2024, so the year matches all rows.
    assert_eq!(find_names(&ordered, "2024"), [
        "Alpha", "beta", "Gamma", "delta"
    ]);
    // Hours and minutes apply to timestamps only.
    assert_eq!(find_names(&ordered, "2024-01-01 10"), ["beta"]);
    assert_eq!(find_names(&ordered, "2024-01-01T11:00"), ["Gamma"]);
    assert_eq!(
        find_names(&ordered, "2024-01-01 11:01"),
        Vec::<String>::new()
    );
    // Not a date: skipped, not an error.
    assert_eq!(find_names(&ordered, "2024-13"), Vec::<String>::new());
    std::fs::remove_file(path).ok();
}

#[test]
fn contains_filter_is_for_text_only() {
    let (path, doc) = temporal_sample();
    let before = doc.unfiltered().unsorted();
    // Numbers and dates are filtered by range; "contains" would mean
    // something different for them, so it is refused.
    for column in [ColumnId::from_raw(1), ColumnId::from_raw(3)] {
        let err = doc
            .filter(&[ColumnFilter {
                column,
                filter: Filter::contains("1700".into(), false, false),
            }])
            .err()
            .unwrap();
        assert!(matches!(err, Error::Filter(_)), "{err}");
    }
    assert_eq!(names(&doc.unfiltered().unsorted()), names(&before));
    std::fs::remove_file(path).ok();
}

#[test]
fn dates_format_parse_and_filter() {
    let (path, dataset) = temporal_sample();
    let c1 = ColumnId::from_raw(1);
    let c2 = ColumnId::from_raw(2);
    let initial = dataset.unfiltered().unsorted();
    assert_eq!(
        initial.cell_text(1, 1, &[c1]).unwrap().cell(0, 0).unwrap(),
        "2024-01-02"
    );

    assert!(dataset
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::range(Some(("not a date").to_owned()), None, false),
        }])
        .is_err());

    assert_eq!(
        initial.cell_text(0, 1, &[c2]).unwrap().cell(0, 0).unwrap(),
        "2024-01-01T09:00:00Z"
    );

    // Range on dates: nulls excluded, bounds inclusive.
    let min = ("2024-01-02").to_owned();
    let doc = dataset
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::range(Some(min), None, false),
        }])
        .unwrap()
        .unsorted();
    assert_eq!(names(&doc), ["beta", "delta"]);

    // Range on timestamps.
    let max = ("2024-01-01T10:00:00Z").to_owned();
    let filtered = dataset
        .filter(&[ColumnFilter {
            column: c2,
            filter: Filter::range(None, Some(max), false),
        }])
        .unwrap();
    assert_eq!(names(&filtered.unsorted()), ["Alpha", "beta"]);

    // Dates are integers: the histogram ends one day past the last date.
    let h = filtered
        .filter_distribution(c1, 60, gewu_core::AxisScale::Linear)
        .map(|result| result.expect("histogram domain").histogram)
        .unwrap();
    assert!(h.integer);
    assert_eq!((h.lo, h.hi), (Some(19_723.0), Some(19_725.0)));
    assert_eq!(filtered.count_nulls(c1).unwrap(), 0);
    std::fs::remove_file(path).ok();
}

#[test]
fn find_marks_matching_cells() {
    let (path, doc) = temporal_sample();
    let cols: Vec<ColumnId> = (0..5).map(ColumnId::from_raw).collect();
    let ordered = doc.unfiltered().unsorted();
    // "170" matches n = 170 only; "2024-01-02" matches the day column.
    for (query, expected) in [("170", vec![(1, 3)]), ("2024-01-02", vec![(1, 1)])] {
        let find = ordered.find(query, &cols).unwrap();
        let hits: Vec<(u64, u32)> = find
            .next(None, 10)
            .unwrap()
            .into_iter()
            .map(|hit| (hit.display_row, hit.column.into_raw()))
            .collect();
        assert_eq!(hits, expected, "query {query}");
    }
    std::fs::remove_file(path).ok();
}
