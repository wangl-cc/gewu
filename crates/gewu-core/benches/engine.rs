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
use gewu_core::{Cancellation, Filter, QuerySpec};

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

fn col(doc: &std::sync::Arc<gewu_core::View>, name: &str) -> u32 {
    doc.dataset()
        .column_index(name)
        .unwrap_or_else(|| panic!("bench file has no column {name:?}"))
}

fn open(c: &mut Criterion) {
    let path = bench_file();
    let mut g = c.benchmark_group("open");
    g.sample_size(10);
    g.bench_function("parquet_2m", |b| {
        b.iter(|| {
            gewu_core::Dataset::open(&path)
                .map(|dataset| std::sync::Arc::new(dataset).view())
                .unwrap()
        })
    });
    g.finish();
}

fn apply_view(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap();
    let (price, score, rating, label, city) = (
        col(&doc, "price"),
        col(&doc, "score"),
        col(&doc, "rating"),
        col(&doc, "label"),
        col(&doc, "city"),
    );
    let (amount, uuid) = (col(&doc, "amount"), col(&doc, "uuid"));
    // Build the label index once, so InValues measures filtering only.
    doc.filter_choices(label, &Cancellation::default()).unwrap();

    let cases: Vec<(&str, QuerySpec)> = vec![
        ("no_filter", QuerySpec::default()),
        (
            "range_1",
            QuerySpec::default().with_filter(
                price,
                Filter::range(Some("100".into()), Some("5000".into()), false),
            ),
        ),
        (
            "range_3",
            QuerySpec::default()
                .with_filter(
                    price,
                    Filter::range(Some("100".into()), Some("5000".into()), false),
                )
                .with_filter(
                    score,
                    Filter::range(Some("80".into()), Some("120".into()), false),
                )
                .with_filter(
                    rating,
                    Filter::range(Some("20".into()), Some("40".into()), false),
                ),
        ),
        (
            "in_values_label",
            QuerySpec::default()
                .with_filter(label, Filter::exact(vec!["cat".into()], false, false)),
        ),
        (
            "contains_city",
            QuerySpec::default()
                .with_filter(city, Filter::contains("city-12".into(), false, false)),
        ),
        (
            "is_null_label",
            QuerySpec::default().with_filter(label, Filter::exact(vec![], false, true)),
        ),
        ("sort_price", QuerySpec::new().with_sort(price, true, 0)),
        (
            "sort_city_price",
            QuerySpec::new()
                .with_sort(city, false, 0)
                .with_sort(price, true, 1),
        ),
        (
            "range_decimal",
            QuerySpec::default().with_filter(
                amount,
                Filter::range(Some("50".into()), Some("500".into()), false),
            ),
        ),
        ("sort_uuid", QuerySpec::new().with_sort(uuid, false, 0)),
        (
            "range_then_sort",
            QuerySpec::default()
                .with_filter(
                    price,
                    Filter::range(Some("100".into()), Some("5000".into()), false),
                )
                .with_sort(score, false, 0),
        ),
    ];

    let mut g = c.benchmark_group("apply_view");
    g.sample_size(20).measurement_time(Duration::from_secs(5));
    for (name, spec) in &cases {
        g.bench_function(*name, |b| {
            b.iter(|| {
                doc.query(black_box(spec), &gewu_core::Cancellation::default())
                    .unwrap()
            })
        });
    }
    g.finish();
}

fn rows(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap();
    let columns: Vec<u32> = (0..doc.dataset().info().columns.len() as u32).collect();
    let mut g = c.benchmark_group("rows");

    let plain = doc
        .query(&QuerySpec::default(), &gewu_core::Cancellation::default())
        .unwrap();
    g.bench_function("block_256_all_columns", |b| {
        b.iter(|| plain.rows(1_000_000, 256, &columns).unwrap())
    });

    let find = plain.find("2024-03", &columns).unwrap();
    g.bench_function("find_highlight_256_all_columns", |b| {
        b.iter(|| {
            find.highlight(1_000, 256, &Cancellation::default())
                .unwrap()
        })
    });
    g.finish();
}

fn facets(c: &mut Criterion) {
    let path = bench_file();
    let doc = gewu_core::Dataset::open(&path)
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap();
    let (label, city, id, price) = (
        col(&doc, "label"),
        col(&doc, "city"),
        col(&doc, "id"),
        col(&doc, "price"),
    );
    let other_filter = QuerySpec::single(
        price,
        Filter::range(Some("100".into()), Some("5000".into()), false),
    );

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
                        .map(|dataset| std::sync::Arc::new(dataset).view())
                        .unwrap()
                },
                |doc| {
                    doc.filter_choices(column, &Cancellation::default())
                        .unwrap()
                },
                BatchSize::PerIteration,
            )
        });
    }
    // Hot: index cached; counts only rows passing the other columns' filters.
    doc.filter_choices(city, &Cancellation::default()).unwrap();
    g.bench_function("choices_hot_city_filtered", |b| {
        b.iter(|| {
            doc.query(&other_filter, &gewu_core::Cancellation::default())
                .unwrap()
                .filter_choices(city, &Cancellation::default())
                .unwrap()
        })
    });

    g.bench_function("histogram_price", |b| {
        b.iter(|| {
            doc.filter_distribution(
                price,
                60,
                gewu_core::AxisScale::Linear,
                &Cancellation::default(),
            )
            .map(|result| result.expect("histogram domain").histogram)
            .unwrap()
        })
    });
    let label_filter = QuerySpec::single(label, Filter::exact(vec![], true, false));
    g.bench_function("histogram_price_filtered", |b| {
        b.iter(|| {
            doc.query(&label_filter, &gewu_core::Cancellation::default())
                .unwrap()
                .filter_distribution(
                    price,
                    60,
                    gewu_core::AxisScale::Linear,
                    &Cancellation::default(),
                )
                .map(|result| result.expect("histogram domain").histogram)
                .unwrap()
        })
    });

    doc.query(&QuerySpec::default(), &gewu_core::Cancellation::default())
        .unwrap();

    let cancellation = Cancellation::default();
    g.bench_function("statistics_price", |b| {
        b.iter(|| doc.analyze_statistics(price, &cancellation).unwrap())
    });
    g.finish();
}

fn inspector_values(c: &mut Criterion) {
    let doc = gewu_core::Dataset::open(bench_file())
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap();
    let mut group = c.benchmark_group("inspector_values");
    group.sample_size(10);
    for name in ["label", "id", "uuid"] {
        let column = col(&doc, name);
        let cancellation = Cancellation::default();
        group.bench_function(name, |b| {
            b.iter(|| doc.analyze_values(column, &cancellation).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, open, apply_view, rows, facets, inspector_values);
criterion_main!(benches);
