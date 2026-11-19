//! Benchmarks for the data engine on a generated 2M-row file.
//!
//! Generate the sample using the Core README, then run `cargo bench`. To use another file, set
//! `GEWU_BENCH_FILE`.
//!
//! "cold" benchmarks open a fresh document for every iteration, so they
//! include one-time work such as building a column's value index; "hot"
//! ones reuse a document where that work is already cached.

use std::{hint::black_box, path::PathBuf, time::Duration};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use gewu_core::{ColumnFilter, ColumnId, ColumnSort, Filter};

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

fn col(doc: &std::sync::Arc<gewu_core::Dataset>, name: &str) -> ColumnId {
    doc.column_id(name)
        .unwrap_or_else(|| panic!("bench file has no column {name:?}"))
}

fn open(c: &mut Criterion) {
    let path = bench_file();
    let mut g = c.benchmark_group("open");
    g.sample_size(10);
    g.bench_function("parquet_2m", |b| {
        b.iter(|| {
            gewu_core::Dataset::open(&path)
                .map(std::sync::Arc::new)
                .unwrap()
        })
    });
    g.finish();
}

fn apply_view(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(std::sync::Arc::new)
        .unwrap();
    let (price, score, rating, label, city) = (
        col(&doc, "price"),
        col(&doc, "score"),
        col(&doc, "rating"),
        col(&doc, "label"),
        col(&doc, "city"),
    );
    let (amount, uuid, country) = (col(&doc, "amount"), col(&doc, "uuid"), col(&doc, "country"));
    // Build low-cardinality dictionaries once, so value filters measure filtering only.
    let unfiltered = doc.unfiltered();
    unfiltered.filter_choices(label).unwrap();
    unfiltered.filter_choices(country).unwrap();

    let cases: Vec<(&str, Vec<ColumnFilter>, Vec<ColumnSort>)> = vec![
        ("no_filter", vec![], vec![]),
        (
            "range_1",
            vec![ColumnFilter {
                column: price,
                filter: Filter::range(Some("100".into()), Some("5000".into()), false),
            }],
            vec![],
        ),
        (
            "range_3",
            vec![
                ColumnFilter {
                    column: price,
                    filter: Filter::range(Some("100".into()), Some("5000".into()), false),
                },
                ColumnFilter {
                    column: score,
                    filter: Filter::range(Some("80".into()), Some("120".into()), false),
                },
                ColumnFilter {
                    column: rating,
                    filter: Filter::range(Some("20".into()), Some("40".into()), false),
                },
            ],
            vec![],
        ),
        (
            "in_values_label",
            vec![ColumnFilter {
                column: label,
                filter: Filter::exact(vec!["cat".into()], false, false),
            }],
            vec![],
        ),
        (
            "contains_city",
            vec![ColumnFilter {
                column: city,
                filter: Filter::contains("city-12".into(), false, false),
            }],
            vec![],
        ),
        (
            "contains_label",
            vec![ColumnFilter {
                column: label,
                filter: Filter::contains("o".into(), false, false),
            }],
            vec![],
        ),
        (
            "contains_country",
            vec![ColumnFilter {
                column: country,
                filter: Filter::contains("a".into(), false, false),
            }],
            vec![],
        ),
        (
            "is_null_label",
            vec![ColumnFilter {
                column: label,
                filter: Filter::exact(vec![], false, true),
            }],
            vec![],
        ),
        ("sort_price", vec![], vec![ColumnSort {
            column: price,
            descending: true,
        }]),
        ("sort_city_price", vec![], vec![
            ColumnSort {
                column: city,
                descending: false,
            },
            ColumnSort {
                column: price,
                descending: true,
            },
        ]),
        (
            "range_decimal",
            vec![ColumnFilter {
                column: amount,
                filter: Filter::range(Some("50".into()), Some("500".into()), false),
            }],
            vec![],
        ),
        ("sort_uuid", vec![], vec![ColumnSort {
            column: uuid,
            descending: false,
        }]),
        (
            "range_then_sort",
            vec![ColumnFilter {
                column: price,
                filter: Filter::range(Some("100".into()), Some("5000".into()), false),
            }],
            vec![ColumnSort {
                column: score,
                descending: false,
            }],
        ),
    ];

    let mut g = c.benchmark_group("apply_view");
    g.sample_size(20).measurement_time(Duration::from_secs(5));
    for (name, filters, sorts) in &cases {
        g.bench_function(*name, |b| {
            b.iter(|| {
                let filtered = doc.filter(black_box(filters)).unwrap();
                filtered.sort(black_box(sorts)).unwrap()
            })
        });
    }
    g.finish();
}

fn rows(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(std::sync::Arc::new)
        .unwrap();
    let columns: Vec<ColumnId> = doc.info().columns.iter().map(|c| c.id).collect();
    let mut g = c.benchmark_group("rows");

    let plain = doc.unfiltered().unsorted();
    g.bench_function("block_256_all_columns", |b| {
        b.iter(|| plain.cell_text(1_000_000, 256, &columns).unwrap())
    });

    // The table's viewport read: one 64-row tile per column and block.
    let tiles: Vec<_> = columns
        .iter()
        .flat_map(|&column| {
            (0..4).map(move |block| gewu_core::CellTile {
                column,
                start: 1_000_000 + block * 64,
                count: 64,
            })
        })
        .collect();
    g.bench_function("tiles_256_all_columns", |b| {
        b.iter(|| plain.read_tiles(&[], &tiles).unwrap())
    });

    let find = plain.find("2024-03", &columns).unwrap();
    g.bench_function("find_highlight_256_all_columns", |b| {
        b.iter(|| find.highlight(1_000, 256).unwrap())
    });
    g.finish();
}

fn facets(c: &mut Criterion) {
    let path = bench_file();
    let doc = gewu_core::Dataset::open(&path)
        .map(std::sync::Arc::new)
        .unwrap();
    let (label, city, id, price) = (
        col(&doc, "label"),
        col(&doc, "city"),
        col(&doc, "id"),
        col(&doc, "price"),
    );
    let other_filter = vec![ColumnFilter {
        column: price,
        filter: Filter::range(Some("100".into()), Some("5000".into()), false),
    }];

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
        g.bench_function(format!("choices_cold_{name}"), |b| {
            b.iter_batched(
                || {
                    gewu_core::Dataset::open(&path)
                        .map(std::sync::Arc::new)
                        .unwrap()
                },
                |doc| doc.unfiltered().filter_choices(column).unwrap(),
                BatchSize::PerIteration,
            )
        });
    }
    // Hot: index cached; counts only rows passing the other columns' filters.
    doc.unfiltered().filter_choices(city).unwrap();
    g.bench_function("choices_hot_city_filtered", |b| {
        b.iter(|| {
            doc.filter(&other_filter)
                .unwrap()
                .filter_choices(city)
                .unwrap()
        })
    });

    g.bench_function("histogram_price", |b| {
        b.iter(|| {
            doc.unfiltered()
                .filter_distribution(price, 60, gewu_core::AxisScale::Linear)
                .map(|result| result.expect("histogram domain").histogram)
                .unwrap()
        })
    });
    let label_filter = vec![ColumnFilter {
        column: label,
        filter: Filter::exact(vec![], true, false),
    }];
    g.bench_function("histogram_price_filtered", |b| {
        b.iter(|| {
            doc.filter(&label_filter)
                .unwrap()
                .filter_distribution(price, 60, gewu_core::AxisScale::Linear)
                .map(|result| result.expect("histogram domain").histogram)
                .unwrap()
        })
    });

    g.bench_function("statistics_price", |b| {
        b.iter(|| doc.unfiltered().statistics(price).unwrap())
    });
    g.finish();
}

fn inspector_values(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(std::sync::Arc::new)
        .unwrap();
    let mut group = c.benchmark_group("inspector_values");
    group.sample_size(10);
    for name in ["label", "id", "uuid"] {
        let column = col(&doc, name);
        group.bench_function(name, |b| {
            b.iter(|| doc.unfiltered().values(column).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, open, apply_view, rows, facets, inspector_values);
criterion_main!(benches);
