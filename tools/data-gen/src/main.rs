//! Write a synthetic Parquet file for manual and performance testing.
//!
//!     cargo run --release -p gewu-data-gen -- out.parquet 5000000 [seed]
//!
//! Values come from a seeded RNG, so the same row count and seed always give
//! the same file. Columns cover the types and shapes the viewer has to
//! handle (see `columns()`): integers and floats of several widths and
//! distributions, decimals, low- and high-cardinality text, dictionary text,
//! booleans, dates, times, timestamps with a time zone, durations, binary,
//! lists and a struct, with nulls, NaN and infinities where they matter.

use std::sync::Arc;

use arrow::{
    array::{
        ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, DictionaryArray,
        DurationMillisecondArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
        Int8Array, ListBuilder, RecordBatch, StringArray, StringBuilder, StructArray,
        Time64MicrosecondArray, TimestampMicrosecondArray, UInt32Array, UInt64Array,
    },
    datatypes::{DataType, Field, Int32Type},
};
use parquet::{
    arrow::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use rand_distr::{Distribution, Exp, LogNormal, Normal, Poisson, Zipf};

const LABELS: [&str; 8] = [
    "cat", "dog", "bird", "fish", "horse", "lizard", "frog", "owl",
];
const WORDS: [&str; 16] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa",
];
const COUNTRIES: [&str; 6] = ["CN", "US", "DE", "JP", "BR", "IN"];
/// 2020-01-01 as days since the epoch.
const DAY_2020: i32 = 18_262;
/// 2024-01-01T00:00:00Z in seconds.
const SECS_2024: i64 = 1_704_067_200;

/// One batch of one column.
type Gen = fn(&mut StdRng, usize, usize) -> ArrayRef;

/// `Some(v)` except for about `rate` of rows.
fn maybe<T>(rng: &mut StdRng, rate: f64, v: T) -> Option<T> {
    (!rng.random_bool(rate)).then_some(v)
}

fn columns() -> Vec<(&'static str, Gen)> {
    vec![
        // --- Columns the benchmarks and earlier tests rely on; keep their meaning.
        // Unique, sequential.
        ("id", |_, start, len| {
            Arc::new(Int64Array::from_iter_values(
                (start..start + len).map(|i| i as i64),
            ))
        }),
        // 8 values, uniform, 2% null.
        ("label", |rng, _, len| {
            Arc::new(StringArray::from_iter((0..len).map(|_| {
                let v = LABELS[rng.random_range(0..LABELS.len())];
                maybe(rng, 0.02, v)
            })))
        }),
        // Uniform 0..10000 with cents.
        ("price", |rng, _, len| {
            Arc::new(Float64Array::from_iter_values(
                (0..len).map(|_| (rng.random_range(0..1_000_000) as f64) / 100.0),
            ))
        }),
        // 2000 values, uniform.
        ("city", |rng, _, len| {
            Arc::new(StringArray::from_iter_values(
                (0..len).map(|_| format!("city-{:04}", rng.random_range(0..2000))),
            ))
        }),
        // Uniform over 2020..2025, 2.5% null.
        ("day", |rng, _, len| {
            Arc::new(Date32Array::from_iter((0..len).map(|_| {
                let d = DAY_2020 + rng.random_range(0..2_192);
                maybe(rng, 0.025, d)
            })))
        }),
        // UTC timestamps across 2024, more during working hours.
        ("updated", |rng, _, len| {
            let hours = [9, 10, 11, 13, 14, 15, 16, 20, 2];
            Arc::new(
                TimestampMicrosecondArray::from_iter_values((0..len).map(|_| {
                    let day = rng.random_range(0..366i64);
                    let hour = hours[rng.random_range(0..hours.len())];
                    let second = rng.random_range(0..3_600i64);
                    (SECS_2024 + day * 86_400 + hour * 3_600 + second) * 1_000_000
                }))
                .with_timezone("UTC"),
            )
        }),
        // Normal(100, 15), 1% null.
        ("score", |rng, _, len| {
            let d = Normal::new(100.0, 15.0).unwrap();
            Arc::new(Float64Array::from_iter((0..len).map(|_| {
                let v = (d.sample(rng) * 100.0f64).round() / 100.0;
                maybe(rng, 0.01, v)
            })))
        }),
        // Integer Normal(30, 5): narrow, one histogram bar per value.
        ("rating", |rng, _, len| {
            let d = Normal::new(30.0f64, 5.0).unwrap();
            Arc::new(Int32Array::from_iter_values(
                (0..len).map(|_| d.sample(rng).round() as i32),
            ))
        }),
        // Mix of N(40, 5) and N(120, 10), 1% null.
        ("bimodal", |rng, _, len| {
            let (a, b) = (
                Normal::new(40.0, 5.0).unwrap(),
                Normal::new(120.0, 10.0).unwrap(),
            );
            Arc::new(Float64Array::from_iter((0..len).map(|_| {
                let v: f64 = if rng.random_bool(0.5) {
                    a.sample(rng)
                } else {
                    b.sample(rng)
                };
                maybe(rng, 0.01, (v * 100.0).round() / 100.0)
            })))
        }),
        // Right-skewed, median 50, 1% null.
        ("lognormal", |rng, _, len| {
            let d = LogNormal::new(50f64.ln(), 0.6).unwrap();
            Arc::new(Float64Array::from_iter((0..len).map(|_| {
                let v = (d.sample(rng) * 100.0f64).round() / 100.0;
                maybe(rng, 0.01, v)
            })))
        }),
        // --- More types and shapes.
        // Exponential waiting times in seconds, f32.
        ("wait_s", |rng, _, len| {
            let d = Exp::new(1.0 / 30.0).unwrap();
            Arc::new(Float32Array::from_iter_values(
                (0..len).map(|_| d.sample(rng) as f32),
            ))
        }),
        // Small counts, Poisson(3), u32.
        ("visits", |rng, _, len| {
            let d = Poisson::new(3.0).unwrap();
            Arc::new(UInt32Array::from_iter_values(
                (0..len).map(|_| d.sample(rng) as u32),
            ))
        }),
        // Heavy-tailed popularity: Zipf over 1000 product ids, int16.
        ("product", |rng, _, len| {
            let d = Zipf::new(1000.0, 1.2).unwrap();
            Arc::new(Int16Array::from_iter_values(
                (0..len).map(|_| d.sample(rng) as i16),
            ))
        }),
        // Tiny signed range, int8, including negatives.
        ("delta", |rng, _, len| {
            Arc::new(Int8Array::from_iter_values(
                (0..len).map(|_| rng.random_range(-5..=5)),
            ))
        }),
        // Large unsigned values above i64::MAX.
        ("hash", |rng, _, len| {
            Arc::new(UInt64Array::from_iter_values(
                (0..len).map(|_| rng.random()),
            ))
        }),
        // Money as decimal(12, 2), log-normal amounts, 3% null.
        ("amount", |rng, _, len| {
            let d = LogNormal::new(4.0, 1.0).unwrap();
            Arc::new(
                Decimal128Array::from_iter((0..len).map(|_| {
                    let cents = (d.sample(rng) * 100.0) as i128;
                    maybe(rng, 0.03, cents)
                }))
                .with_precision_and_scale(12, 2)
                .unwrap(),
            )
        }),
        // Sensor readings with NaN and infinities mixed in.
        ("sensor", |rng, _, len| {
            let d = Normal::new(0.0, 1.0).unwrap();
            Arc::new(Float64Array::from_iter_values((0..len).map(
                |_| match rng.random_range(0..1000) {
                    0 => f64::NAN,
                    1 => f64::INFINITY,
                    2 => f64::NEG_INFINITY,
                    _ => d.sample(rng),
                },
            )))
        }),
        // True about 30% of the time, 5% null.
        ("active", |rng, _, len| {
            Arc::new(BooleanArray::from_iter((0..len).map(|_| {
                let v = rng.random_bool(0.3);
                maybe(rng, 0.05, v)
            })))
        }),
        // Dictionary-encoded text, as Parquet writers often produce.
        ("country", |rng, _, len| {
            let keys = Int32Array::from_iter_values(
                (0..len).map(|_| rng.random_range(0..COUNTRIES.len() as i32)),
            );
            let values = Arc::new(StringArray::from(COUNTRIES.to_vec()));
            Arc::new(DictionaryArray::<Int32Type>::try_new(keys, values).unwrap())
        }),
        // Unique, UUID-like text: high cardinality.
        ("uuid", |rng, _, len| {
            Arc::new(StringArray::from_iter_values((0..len).map(|_| {
                let v: u128 = rng.random();
                format!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    (v >> 96) as u32,
                    (v >> 80) as u16,
                    (v >> 64) as u16,
                    (v >> 48) as u16,
                    v & 0xffff_ffff_ffff
                )
            })))
        }),
        // Free text of varying length, 5% null.
        ("note", |rng, _, len| {
            Arc::new(StringArray::from_iter((0..len).map(|_| {
                let words = rng.random_range(1..12);
                let text = (0..words)
                    .map(|_| WORDS[rng.random_range(0..WORDS.len())])
                    .collect::<Vec<_>>()
                    .join(" ");
                maybe(rng, 0.05, text)
            })))
        }),
        // Time of day, microseconds.
        ("start_time", |rng, _, len| {
            Arc::new(Time64MicrosecondArray::from_iter_values(
                (0..len).map(|_| rng.random_range(0..86_400_000_000i64)),
            ))
        }),
        // Durations in milliseconds, exponential.
        ("latency", |rng, _, len| {
            let d = Exp::new(1.0 / 250.0).unwrap();
            Arc::new(DurationMillisecondArray::from_iter_values(
                (0..len).map(|_| d.sample(rng) as i64),
            ))
        }),
        // Short binary blobs.
        ("blob", |rng, _, len| {
            let values: Vec<Vec<u8>> = (0..len)
                .map(|_| (0..rng.random_range(0..16)).map(|_| rng.random()).collect())
                .collect();
            Arc::new(BinaryArray::from_iter_values(values.iter()))
        }),
        // Lists of 0..4 tags, 2% null lists.
        ("tags", |rng, _, len| {
            let mut b = ListBuilder::new(StringBuilder::new());
            for _ in 0..len {
                if rng.random_bool(0.02) {
                    b.append(false);
                    continue;
                }
                for _ in 0..rng.random_range(0..4) {
                    b.values()
                        .append_value(WORDS[rng.random_range(0..WORDS.len())]);
                }
                b.append(true);
            }
            Arc::new(b.finish())
        }),
        // A nested struct.
        ("location", |rng, _, len| {
            let lat: ArrayRef = Arc::new(Float64Array::from_iter_values(
                (0..len).map(|_| rng.random_range(-90.0..90.0)),
            ));
            let lon: ArrayRef = Arc::new(Float64Array::from_iter_values(
                (0..len).map(|_| rng.random_range(-180.0..180.0)),
            ));
            Arc::new(StructArray::from(vec![
                (Arc::new(Field::new("lat", DataType::Float64, false)), lat),
                (Arc::new(Field::new("lon", DataType::Float64, false)), lon),
            ]))
        }),
        // Edge cases: one value, and no values at all.
        ("constant", |_, _, len| {
            Arc::new(Int32Array::from_iter_values(std::iter::repeat_n(7, len)))
        }),
        ("empty", |_, _, len| {
            Arc::new(Float64Array::from(vec![None::<f64>; len]))
        }),
    ]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "sample.parquet".into());
    let n: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(1_000_000);
    let seed: u64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(42);

    let columns = columns();
    // One RNG per column, so adding a column never changes the others.
    let mut rngs: Vec<StdRng> = (0..columns.len() as u64)
        .map(|i| StdRng::seed_from_u64(seed.wrapping_mul(1_000_003).wrapping_add(i)))
        .collect();

    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    let file = std::fs::File::create(&path).unwrap();
    let mut writer: Option<ArrowWriter<std::fs::File>> = None;
    let mut file = Some(file);

    let batch_size = 256 * 1024;
    for start in (0..n).step_by(batch_size) {
        let len = batch_size.min(n - start);
        let arrays = columns
            .iter()
            .zip(rngs.iter_mut())
            .map(|((name, gen), rng)| (*name, gen(rng, start, len)));
        let batch = RecordBatch::try_from_iter(arrays).unwrap();
        let w = writer.get_or_insert_with(|| {
            ArrowWriter::try_new(file.take().unwrap(), batch.schema(), Some(props.clone())).unwrap()
        });
        w.write(&batch).unwrap();
    }
    writer.unwrap().close().unwrap();
    println!(
        "wrote {n} rows × {} columns to {path} (seed {seed})",
        columns.len()
    );
}
