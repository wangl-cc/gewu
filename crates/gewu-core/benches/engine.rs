//! Benchmarks for the data engine on a generated 2M-row file.
//!
//! Generate the sample using the Core README, then run `cargo bench`. To use another file, set
//! `GEWU_BENCH_FILE`.
//!
//! "cold" benchmarks open a fresh document for every iteration, so they
//! include one-time work such as building a column's value index; "hot"
//! ones reuse a document where that work is already cached.

use std::{path::PathBuf, time::Duration};

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use gewu_core::{AnalysisSection, Document, Filter, SortKey, ViewState};

fn bench_file() -> PathBuf {
    let path = std::env::var_os("GEWU_BENCH_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/bench/sample.parquet")
        });
    assert!(
        path.exists(),
        "{} not found: generate the sample using the Core README, or set GEWU_BENCH_FILE",
        path.display()
    );
    path
}

fn col(doc: &Document, name: &str) -> u32 {
    doc.column_index(name)
        .unwrap_or_else(|| panic!("bench file has no column {name:?}"))
}

fn search(text: &str) -> ViewState {
    ViewState {
        search: Some(text.into()),
        ..Default::default()
    }
}

fn open(c: &mut Criterion) {
    let path = bench_file();
    let mut g = c.benchmark_group("open");
    g.sample_size(10);
    g.bench_function("parquet_2m", |b| b.iter(|| Document::open(&path).unwrap()));
    g.finish();
}

fn apply_view(c: &mut Criterion) {
    let doc = Document::open(bench_file()).unwrap();
    let (price, score, rating, label, city) = (
        col(&doc, "price"),
        col(&doc, "score"),
        col(&doc, "rating"),
        col(&doc, "label"),
        col(&doc, "city"),
    );
    let (amount, uuid) = (col(&doc, "amount"), col(&doc, "uuid"));
    // Build the label index once, so InValues measures filtering only.
    let cat = doc.keys_for_labels(label, &[Some("cat".into())]).unwrap();

    let cases: Vec<(&str, ViewState)> = vec![
        ("no_filter", ViewState::default()),
        ("range_1", ViewState {
            filters: vec![Filter::Range {
                include_nulls: false,
                column: price,
                min: Some(doc.make_bound_i64(price, 100).unwrap()),
                max: Some(doc.make_bound_i64(price, 5000).unwrap()),
            }],
            ..Default::default()
        }),
        ("range_3", ViewState {
            filters: vec![
                Filter::Range {
                    include_nulls: false,
                    column: price,
                    min: Some(doc.make_bound_i64(price, 100).unwrap()),
                    max: Some(doc.make_bound_i64(price, 5000).unwrap()),
                },
                Filter::Range {
                    include_nulls: false,
                    column: score,
                    min: Some(doc.make_bound_i64(score, 80).unwrap()),
                    max: Some(doc.make_bound_i64(score, 120).unwrap()),
                },
                Filter::Range {
                    include_nulls: false,
                    column: rating,
                    min: Some(doc.make_bound_i64(rating, 20).unwrap()),
                    max: Some(doc.make_bound_i64(rating, 40).unwrap()),
                },
            ],
            ..Default::default()
        }),
        ("in_values_label", ViewState {
            filters: vec![Filter::InValues {
                column: label,
                keys: cat,
            }],
            ..Default::default()
        }),
        ("contains_city", ViewState {
            filters: vec![Filter::Contains {
                include_nulls: false,
                column: city,
                text: "city-12".into(),
                case_sensitive: false,
            }],
            ..Default::default()
        }),
        ("is_null_label", ViewState {
            filters: vec![Filter::IsNull {
                column: label,
                negate: false,
            }],
            ..Default::default()
        }),
        ("search_text", search("city-12")),
        ("search_int", search("123456")),
        ("search_float", search("3089.62")),
        ("search_date", search("2024-03")),
        ("search_uuid_prefix", search("0000a")),
        ("search_bool", search("true")),
        ("sort_price", ViewState {
            sort: vec![SortKey {
                column: price,
                descending: true,
            }],
            ..Default::default()
        }),
        ("sort_city_price", ViewState {
            sort: vec![
                SortKey {
                    column: city,
                    descending: false,
                },
                SortKey {
                    column: price,
                    descending: true,
                },
            ],
            ..Default::default()
        }),
        ("range_decimal", ViewState {
            filters: vec![Filter::Range {
                include_nulls: false,
                column: amount,
                min: Some(doc.make_bound_i64(amount, 50).unwrap()),
                max: Some(doc.make_bound_i64(amount, 500).unwrap()),
            }],
            ..Default::default()
        }),
        ("sort_uuid", ViewState {
            sort: vec![SortKey {
                column: uuid,
                descending: false,
            }],
            ..Default::default()
        }),
        ("range_then_sort", ViewState {
            filters: vec![Filter::Range {
                include_nulls: false,
                column: price,
                min: Some(doc.make_bound_i64(price, 100).unwrap()),
                max: Some(doc.make_bound_i64(price, 5000).unwrap()),
            }],
            sort: vec![SortKey {
                column: score,
                descending: false,
            }],
            ..Default::default()
        }),
    ];

    let mut g = c.benchmark_group("apply_view");
    g.sample_size(20).measurement_time(Duration::from_secs(5));
    for (name, state) in &cases {
        g.bench_function(*name, |b| {
            b.iter(|| doc.apply_view(black_box(state)).unwrap())
        });
    }
    g.finish();
}

fn rows(c: &mut Criterion) {
    let doc = Document::open(bench_file()).unwrap();
    let columns: Vec<u32> = (0..doc.info().columns.len() as u32).collect();
    let mut g = c.benchmark_group("rows");

    let plain = doc.apply_view(&ViewState::default()).unwrap();
    g.bench_function("block_256_all_columns", |b| {
        b.iter(|| doc.rows(plain.version, 1_000_000, 256, &columns).unwrap())
    });

    // With a search, rows() also marks which cells match.
    let searched = doc.apply_view(&search("2024-03")).unwrap();
    g.bench_function("block_256_with_highlight", |b| {
        b.iter(|| doc.rows(searched.version, 1_000, 256, &columns).unwrap())
    });
    g.finish();
}

fn facets(c: &mut Criterion) {
    let path = bench_file();
    let doc = Document::open(&path).unwrap();
    let (label, city, id, price) = (
        col(&doc, "label"),
        col(&doc, "city"),
        col(&doc, "id"),
        col(&doc, "price"),
    );
    let other_filter = ViewState {
        filters: vec![Filter::Range {
            include_nulls: false,
            column: price,
            min: Some(doc.make_bound_i64(price, 100).unwrap()),
            max: Some(doc.make_bound_i64(price, 5000).unwrap()),
        }],
        ..Default::default()
    };

    let mut g = c.benchmark_group("facets");
    g.sample_size(10).measurement_time(Duration::from_secs(8));

    // Cold: includes building the column's value index.
    let uuid = col(&doc, "uuid");
    for (name, column) in [
        ("label", label),
        ("city", city),
        ("id_high_card", id),
        ("uuid_high_card", uuid),
    ] {
        g.bench_function(format!("distinct_cold_{name}"), |b| {
            b.iter_batched(
                || Document::open(&path).unwrap(),
                |doc| doc.distinct_values(column, None, 10_000).unwrap(),
                BatchSize::PerIteration,
            )
        });
    }
    // Hot: index cached; counts only rows passing the other columns' filters.
    doc.distinct_values(city, None, 1).unwrap();
    g.bench_function("distinct_hot_city_filtered", |b| {
        b.iter(|| {
            doc.distinct_values(city, Some(&other_filter), 10_000)
                .unwrap()
        })
    });

    g.bench_function("histogram_price", |b| {
        b.iter(|| doc.histogram(price, None, 60).unwrap())
    });
    let label_filter = ViewState {
        filters: vec![Filter::IsNull {
            column: label,
            negate: true,
        }],
        ..Default::default()
    };
    g.bench_function("histogram_price_filtered", |b| {
        b.iter(|| doc.histogram(price, Some(&label_filter), 60).unwrap())
    });

    doc.apply_view(&ViewState::default()).unwrap();
    g.bench_function("summary_price", |b| {
        b.iter(|| doc.column_summary(price, 8).unwrap())
    });
    g.bench_function("summary_label_hot", |b| {
        doc.column_summary(label, 8).unwrap();
        b.iter(|| doc.column_summary(label, 8).unwrap())
    });
    // Null count, distinct count, min and max of a freshly opened column
    // (`column_stats` before it was merged into the summary).
    g.bench_function("stats_price_cold", |b| {
        b.iter_batched(
            || Document::open(&path).unwrap(),
            |doc| doc.column_summary(price, 8).unwrap(),
            BatchSize::PerIteration,
        )
    });
    g.finish();
}

fn inspector_values(c: &mut Criterion) {
    let doc = Document::open(bench_file()).unwrap();
    let mut group = c.benchmark_group("inspector_values");
    group.sample_size(10);
    for name in ["label", "id", "uuid"] {
        let job = doc
            .prepare_analysis(
                doc.current_view().version,
                col(&doc, name),
                AnalysisSection::Values,
            )
            .unwrap();
        group.bench_function(name, |b| b.iter(|| job.run().unwrap()));
    }
    group.finish();
}

criterion_group!(benches, open, apply_view, rows, facets, inspector_values);
criterion_main!(benches);
