//! Speed of the operations the app waits on, over the synthetic sample
//! (`examples/sample`: 20 columns of every kind; 2,000,000 rows by default,
//! set `GEWU_BENCH_ROWS` to change it).
//!
//!     cargo bench -p gewu-core --bench engine
//!     cargo bench -p gewu-core --bench engine -- --save-baseline before
//!     cargo bench -p gewu-core --bench engine -- --baseline before
//!
//! The file is written and opened once; each benchmark times only its own
//! call. Peak memory is measured separately by `scripts/bench-memory.py`.

#[path = "../examples/sample/mod.rs"]
mod sample;

use std::{hint::black_box, time::Duration};

use criterion::{criterion_group, criterion_main, Criterion};
use gewu_core::{Bound, Document, Filter, SortKey, ViewState};

struct Fixture {
    doc: Document,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let rows = std::env::var("GEWU_BENCH_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.parquet");
    sample::write(&path, rows);
    Fixture {
        doc: Document::open(&path).unwrap(),
        _dir: dir,
    }
}

fn col(doc: &Document, name: &str) -> u32 {
    doc.column_index(name).unwrap()
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

fn benches(c: &mut Criterion) {
    let f = fixture();
    let doc = &f.doc;
    let [id, uuid, label, city, country, price, score, amount, day, updated, tags, note] = [
        "id", "uuid", "label", "city", "country", "price", "score", "amount", "day", "updated",
        "tags", "note",
    ]
    .map(|name| col(doc, name));

    let path = doc.info().path.clone();
    c.bench_function("open", |b| {
        b.iter(|| Document::open(black_box(&path)).unwrap())
    });

    let cat_owl = doc
        .keys_for_labels(label, &[Some("cat".into()), Some("owl".into())])
        .unwrap();
    let views: Vec<(&str, ViewState)> = vec![
        ("all", ViewState::default()),
        ("range", ViewState {
            filters: vec![range(price, 1000.0, 5000.0)],
            ..Default::default()
        }),
        ("range_x2", ViewState {
            filters: vec![range(price, 1000.0, 5000.0), range(score, 85.0, 115.0)],
            ..Default::default()
        }),
        ("checkbox", ViewState {
            filters: vec![Filter::InValues {
                column: label,
                keys: cat_owl.clone(),
            }],
            ..Default::default()
        }),
        ("contains", ViewState {
            filters: vec![Filter::Contains {
                column: city,
                text: "-012".into(),
                case_sensitive: false,
            }],
            ..Default::default()
        }),
        ("is_null", ViewState {
            filters: vec![Filter::IsNull {
                column: day,
                negate: false,
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
        ("search_text", search("owl")),
        ("search_number", search("1373.03")),
        ("search_date", search("2024-03")),
        ("sort_float", ViewState {
            sort: vec![SortKey {
                column: price,
                descending: false,
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
        ("sort_text_then_float", ViewState {
            sort: vec![
                SortKey {
                    column: label,
                    descending: false,
                },
                SortKey {
                    column: price,
                    descending: true,
                },
            ],
            ..Default::default()
        }),
        ("filter_and_sort", ViewState {
            filters: vec![range(price, 1000.0, 5000.0), Filter::InValues {
                column: label,
                keys: cat_owl,
            }],
            sort: vec![SortKey {
                column: score,
                descending: true,
            }],
            ..Default::default()
        }),
    ];
    let mut g = c.benchmark_group("apply_view");
    for (name, state) in &views {
        g.bench_function(*name, |b| {
            b.iter(|| doc.apply_view(black_box(state)).unwrap())
        });
    }
    g.finish();

    // Facets for an open filter popover: count with the other filters.
    let facet_state = &views[2].1;
    let mut g = c.benchmark_group("distinct");
    for (name, column) in [
        ("label", label),
        ("city", city),
        ("country_dictionary", country),
        ("tags_list", tags),
        ("note", note),
        ("id", id),
        ("uuid", uuid),
    ] {
        g.bench_function(name, |b| {
            b.iter(|| doc.distinct_values(column, None, 10_000).unwrap())
        });
    }
    g.bench_function("city_with_filters", |b| {
        b.iter(|| {
            doc.distinct_values(city, Some(facet_state), 10_000)
                .unwrap()
        })
    });
    g.finish();

    let mut g = c.benchmark_group("histogram");
    for (name, column) in [
        ("price", price),
        ("decimal", amount),
        ("day", day),
        ("timestamp", updated),
    ] {
        g.bench_function(name, |b| {
            b.iter(|| doc.histogram(column, None, 100).unwrap())
        });
    }
    g.bench_function("score_with_filters", |b| {
        b.iter(|| doc.histogram(score, Some(facet_state), 100).unwrap())
    });
    g.finish();

    // The inspector, over the whole file.
    doc.apply_view(&ViewState::default()).unwrap();
    let mut g = c.benchmark_group("column_summary");
    for (name, column) in [
        ("price", price),
        ("label", label),
        ("city", city),
        ("note", note),
        ("id", id),
        ("uuid", uuid),
    ] {
        g.bench_function(name, |b| b.iter(|| doc.column_summary(column, 10).unwrap()));
    }
    g.finish();

    // Scrolling: one block of rows, all columns, from a sorted view.
    let sorted = doc
        .apply_view(&views.iter().find(|(n, _)| *n == "sort_float").unwrap().1)
        .unwrap();
    let columns: Vec<u32> = (0..doc.info().columns.len() as u32).collect();
    let middle = sorted.row_count / 2;
    c.bench_function("rows_256", |b| {
        b.iter(|| doc.rows(sorted.version, middle, 256, &columns).unwrap())
    });
}

criterion_group! {
    name = engine;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = benches
}
criterion_main!(engine);
