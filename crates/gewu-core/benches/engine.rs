//! Benchmarks for the data engine on a generated 2M-row file.
//!
//! Run with `scripts/bench.sh`, which creates the file (fixed seed, every
//! kind of column; see `examples/gen_sample.rs`) and runs `cargo bench`. To
//! use another file, set `GEWU_BENCH_FILE`.
//!
//! "cold" benchmarks open a fresh document for every iteration, so they
//! include one-time work such as building a column's value index; "hot"
//! ones reuse a document where that work is already cached.

use std::{hint::black_box, path::PathBuf, time::Duration};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use gewu_core::{Bound, Document, Filter, SortKey, ViewState};

fn bench_file() -> PathBuf {
    let path = std::env::var_os("GEWU_BENCH_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("gewu-bench-v2.parquet"));
    assert!(
        path.exists(),
        "{} not found: run scripts/bench.sh, or set GEWU_BENCH_FILE",
        path.display()
    );
    path
}

fn col(doc: &Document, name: &str) -> u32 {
    doc.column_index(name)
        .unwrap_or_else(|| panic!("bench file has no column {name:?}"))
}

fn range(column: u32, min: f64, max: f64) -> Filter {
    Filter::Range {
        column,
        min: Some(Bound::Float(min)),
        max: Some(Bound::Float(max)),
    }
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
    let [price, score, rating, label, city, uuid, updated, tags] = [
        "price", "score", "rating", "label", "city", "uuid", "updated", "tags",
    ]
    .map(|name| col(&doc, name));
    // Build the label index once, so InValues measures filtering only.
    let cat = doc.keys_for_labels(label, &[Some("cat".into())]).unwrap();

    let cases: Vec<(&str, ViewState)> = vec![
        ("no_filter", ViewState::default()),
        ("range_1", ViewState {
            filters: vec![range(price, 100.0, 5000.0)],
            ..Default::default()
        }),
        ("range_3", ViewState {
            filters: vec![
                range(price, 100.0, 5000.0),
                range(score, 80.0, 120.0),
                range(rating, 20.0, 40.0),
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
                column: city,
                text: "city-12".into(),
                case_sensitive: false,
            }],
            ..Default::default()
        }),
        ("contains_list", ViewState {
            filters: vec![Filter::Contains {
                column: tags,
                text: "owl".into(),
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
        ("search_time", search("12:30")),
        ("search_bool", search("true")),
        ("sort_price", ViewState {
            sort: vec![SortKey {
                column: price,
                descending: true,
            }],
            ..Default::default()
        }),
        ("sort_unique_text", ViewState {
            sort: vec![SortKey {
                column: uuid,
                descending: false,
            }],
            ..Default::default()
        }),
        ("sort_timestamp", ViewState {
            sort: vec![SortKey {
                column: updated,
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
        ("range_then_sort", ViewState {
            filters: vec![range(price, 100.0, 5000.0)],
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
    let [label, city, country, tags, note, id, uuid, price, amount, updated] = [
        "label", "city", "country", "tags", "note", "id", "uuid", "price", "amount", "updated",
    ]
    .map(|name| col(&doc, name));
    let other_filter = ViewState {
        filters: vec![range(price, 100.0, 5000.0)],
        ..Default::default()
    };

    let mut g = c.benchmark_group("facets");
    g.sample_size(10).measurement_time(Duration::from_secs(8));

    // Cold: includes building the column's value index.
    for (name, column) in [
        ("label", label),
        ("city", city),
        ("country_dictionary", country),
        ("tags_list", tags),
        ("note_text", note),
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
    g.bench_function("histogram_decimal", |b| {
        b.iter(|| doc.histogram(amount, None, 60).unwrap())
    });
    g.bench_function("histogram_timestamp", |b| {
        b.iter(|| doc.histogram(updated, None, 60).unwrap())
    });

    doc.apply_view(&ViewState::default()).unwrap();
    g.bench_function("summary_price", |b| {
        b.iter(|| doc.column_summary(price, 8).unwrap())
    });
    g.bench_function("summary_label_hot", |b| {
        doc.column_summary(label, 8).unwrap();
        b.iter(|| doc.column_summary(label, 8).unwrap())
    });
    // Cold: the first summary of a column, on a freshly opened file.
    for (name, column) in [("price", price), ("note", note), ("uuid", uuid)] {
        g.bench_function(format!("summary_cold_{name}"), |b| {
            b.iter_batched(
                || {
                    let doc = Document::open(&path).unwrap();
                    doc.apply_view(&ViewState::default()).unwrap();
                    doc
                },
                |doc| doc.column_summary(column, 8).unwrap(),
                BatchSize::PerIteration,
            )
        });
    }
    g.finish();
}

criterion_group!(benches, open, apply_view, rows, facets);
criterion_main!(benches);
