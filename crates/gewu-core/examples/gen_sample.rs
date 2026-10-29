//! Write a synthetic Parquet file for manual and performance testing. The
//! generator has a fixed seed, so the same `n` always gives the same file.
//!
//!     cargo run --release -p gewu-core --example gen_sample -- out.parquet 5000000
//!
//! It covers every kind of column the viewer treats differently, with
//! distributions whose shape is easy to recognise in a histogram:
//!
//! | column    | type                    | values                                    |
//! |-----------|-------------------------|-------------------------------------------|
//! | id        | Int64                   | 0..n, unique                              |
//! | uuid      | Utf8                    | random hex, unique (too many to index)    |
//! | label     | Utf8                    | 8 animals, Zipf-skewed, 2% null           |
//! | city      | Utf8                    | 2,000 cities, uniform                     |
//! | country   | Dictionary(Int32, Utf8) | 40 codes, Zipf-skewed                     |
//! | active    | Boolean                 | 30% true, 1% null                         |
//! | price     | Float64                 | uniform 0–10,000, cents                   |
//! | score     | Float64                 | normal(100, 15), 1% null                  |
//! | bimodal   | Float64                 | normal(40, 5) and normal(120, 10)         |
//! | lognormal | Float64                 | log-normal, median 50                     |
//! | wait      | Float32                 | exponential, mean 2; a few NaN            |
//! | rating    | Int32                   | normal(30, 5), rounded                    |
//! | visits    | UInt64                  | Pareto heavy tail                         |
//! | amount    | Decimal128(12, 2)       | log-normal, median 120.00                 |
//! | day       | Date32                  | 2020–2025 uniform, 2.5% null              |
//! | updated   | Timestamp(µs, UTC)      | 2024, weighted towards working hours      |
//! | clock     | Time64(µs)              | normal around 12:30                       |
//! | elapsed   | Duration(ms)            | exponential, mean 90 s                    |
//! | tags      | List(Utf8)              | 0–3 animals                               |
//! | note      | LargeUtf8               | a few words, 10% null                     |

use std::{path::Path, sync::Arc};

use arrow::{
    array::{
        ArrayRef, BooleanArray, Date32Array, Decimal128Array, DurationMillisecondArray,
        Float32Array, Float64Array, Int32Array, Int64Array, LargeStringArray, ListBuilder,
        RecordBatch, StringArray, StringBuilder, StringDictionaryBuilder, Time64MicrosecondArray,
        TimestampMicrosecondArray, UInt64Array,
    },
    datatypes::Int32Type,
};
use parquet::{
    arrow::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};
use rand::{rngs::Xoshiro256PlusPlus, Rng, RngExt, SeedableRng};
use rand_distr::{Distribution, Exp, LogNormal, Normal, Pareto, Zipf};

const ANIMALS: [&str; 8] = [
    "cat", "dog", "bird", "fish", "horse", "lizard", "frog", "owl",
];
const WORDS: [&str; 16] = [
    "red", "quick", "late", "early", "big", "small", "new", "old", "box", "order", "return",
    "gift", "sale", "repair", "visit", "note",
];

/// Write `n` rows to `path`.
fn write(path: &Path, n: usize) {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x6765_7775);
    let file = std::fs::File::create(path).unwrap();
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    let mut writer: Option<ArrowWriter<std::fs::File>> = None;
    let mut file = Some(file);

    let batch_size = 256 * 1024;
    for start in (0..n).step_by(batch_size) {
        let len = batch_size.min(n - start);
        let batch = batch(&mut rng, start, len);
        let w = writer.get_or_insert_with(|| {
            ArrowWriter::try_new(file.take().unwrap(), batch.schema(), Some(props.clone())).unwrap()
        });
        w.write(&batch).unwrap();
    }
    writer.unwrap().close().unwrap();
}

fn batch(rng: &mut Xoshiro256PlusPlus, start: usize, len: usize) -> RecordBatch {
    let animal = Zipf::new(ANIMALS.len() as f64, 1.0).unwrap();
    let country = Zipf::new(40.0, 1.2).unwrap();
    let score = Normal::new(100.0, 15.0).unwrap();
    let low = Normal::new(40.0, 5.0).unwrap();
    let high = Normal::new(120.0, 10.0).unwrap();
    let lognormal = LogNormal::new(50f64.ln(), 0.6).unwrap();
    let wait = Exp::new(0.5).unwrap();
    let rating = Normal::<f64>::new(30.0, 5.0).unwrap();
    let visits = Pareto::<f64>::new(1.0, 1.5).unwrap();
    let amount = LogNormal::new(120f64.ln(), 0.8).unwrap();
    let clock = Normal::<f64>::new(12.5 * 3600.0, 3.0 * 3600.0).unwrap();
    let elapsed = Exp::new(1.0 / 90_000.0).unwrap();
    let pick = |rng: &mut Xoshiro256PlusPlus| ANIMALS[animal.sample(rng) as usize - 1];
    let rows = start..start + len;
    // Fixed 1-in-97 nulls, independent of the random stream.
    let sometimes = |i: usize| !i.is_multiple_of(97);

    let id: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.clone().map(|i| i as i64)));
    let uuid: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..len).map(|_| format!("{:016x}", rng.next_u64())),
    ));
    let label: ArrayRef = Arc::new(StringArray::from_iter(
        (0..len).map(|_| (!rng.random_bool(0.02)).then(|| pick(rng))),
    ));
    let city: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..len).map(|_| format!("city-{:04}", rng.random_range(0..2000))),
    ));
    let mut countries = StringDictionaryBuilder::<Int32Type>::new();
    for _ in 0..len {
        let c = country.sample(rng) as u8 - 1;
        countries.append_value(format!(
            "{}{}",
            (b'A' + c / 8) as char,
            (b'A' + c % 8) as char
        ));
    }
    let country: ArrayRef = Arc::new(countries.finish());
    let active: ArrayRef = Arc::new(BooleanArray::from_iter(
        rows.clone()
            .map(|i| (!i.is_multiple_of(100)).then(|| rng.random_bool(0.3))),
    ));
    let price: ArrayRef = Arc::new(Float64Array::from_iter_values(
        (0..len).map(|_| rng.random_range(0..1_000_000) as f64 / 100.0),
    ));
    let cents = |v: f64| (v * 100.0).round() / 100.0;
    let score: ArrayRef = Arc::new(Float64Array::from_iter(
        rows.clone()
            .map(|i| sometimes(i).then(|| cents(score.sample(rng)))),
    ));
    let bimodal: ArrayRef = Arc::new(Float64Array::from_iter(rows.clone().map(|i| {
        let v = if rng.random_bool(0.5) {
            low.sample(rng)
        } else {
            high.sample(rng)
        };
        sometimes(i).then(|| cents(v))
    })));
    let lognormal: ArrayRef = Arc::new(Float64Array::from_iter(
        rows.clone()
            .map(|i| sometimes(i).then(|| cents(lognormal.sample(rng)))),
    ));
    let wait: ArrayRef = Arc::new(Float32Array::from_iter_values(rows.clone().map(|i| {
        if i.is_multiple_of(5_000) {
            f32::NAN
        } else {
            wait.sample(rng) as f32
        }
    })));
    let rating: ArrayRef = Arc::new(Int32Array::from_iter_values(
        (0..len).map(|_| rating.sample(rng).round() as i32),
    ));
    let visits: ArrayRef = Arc::new(UInt64Array::from_iter_values(
        (0..len).map(|_| visits.sample(rng).min(1e12) as u64),
    ));
    let amount: ArrayRef = Arc::new(
        Decimal128Array::from_iter_values(
            (0..len).map(|_| (amount.sample(rng) * 100.0).round() as i128),
        )
        .with_precision_and_scale(12, 2)
        .unwrap(),
    );
    // Days from 2020-01-01 (18262) over six years, some missing.
    let day: ArrayRef =
        Arc::new(Date32Array::from_iter((0..len).map(|_| {
            (!rng.random_bool(0.025)).then(|| 18_262 + rng.random_range(0..2_192))
        })));
    let updated: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from_iter_values((0..len).map(|_| {
            let day = rng.random_range(0..366i64);
            let hour = [9, 10, 11, 13, 14, 15, 16, 20, 2][rng.random_range(0..9)];
            let second = rng.random_range(0..3_600i64);
            (1_704_067_200 + day * 86_400 + hour * 3_600 + second) * 1_000_000
        }))
        .with_timezone("UTC"),
    );
    let clock: ArrayRef = Arc::new(Time64MicrosecondArray::from_iter_values((0..len).map(
        |_| {
            let s = clock.sample(rng).clamp(0.0, 86_399.0);
            (s * 1e6) as i64
        },
    )));
    let elapsed: ArrayRef = Arc::new(DurationMillisecondArray::from_iter_values(
        (0..len).map(|_| elapsed.sample(rng) as i64),
    ));
    let mut tags = ListBuilder::new(StringBuilder::new());
    for _ in 0..len {
        for _ in 0..rng.random_range(0..4) {
            tags.values().append_value(pick(rng));
        }
        tags.append(true);
    }
    let tags: ArrayRef = Arc::new(tags.finish());
    let note: ArrayRef = Arc::new(LargeStringArray::from_iter((0..len).map(|_| {
        (!rng.random_bool(0.1)).then(|| {
            let words: Vec<&str> = (0..rng.random_range(1..6))
                .map(|_| WORDS[rng.random_range(0..WORDS.len())])
                .collect();
            words.join(" ")
        })
    })));

    RecordBatch::try_from_iter(vec![
        ("id", id),
        ("uuid", uuid),
        ("label", label),
        ("city", city),
        ("country", country),
        ("active", active),
        ("price", price),
        ("score", score),
        ("bimodal", bimodal),
        ("lognormal", lognormal),
        ("wait", wait),
        ("rating", rating),
        ("visits", visits),
        ("amount", amount),
        ("day", day),
        ("updated", updated),
        ("clock", clock),
        ("elapsed", elapsed),
        ("tags", tags),
        ("note", note),
    ])
    .unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "sample.parquet".into());
    let n: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(1_000_000);
    write(path.as_ref(), n);
    println!("wrote {n} rows to {path}");
}
