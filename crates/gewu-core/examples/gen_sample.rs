//! Write a synthetic Parquet file for manual and performance testing.
//!
//!     cargo run --release -p gewu-core --example gen_sample -- out.parquet 5000000

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

const LABELS: [&str; 8] = [
    "cat", "dog", "bird", "fish", "horse", "lizard", "frog", "owl",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "sample.parquet".into());
    let n: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(1_000_000);

    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let file = std::fs::File::create(&path).unwrap();
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    let mut writer: Option<ArrowWriter<std::fs::File>> = None;
    let mut file = Some(file);

    let batch_size = 256 * 1024;
    for start in (0..n).step_by(batch_size) {
        let len = batch_size.min(n - start);
        let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (start..start + len).map(|i| i as i64),
        ));
        let labels: ArrayRef = Arc::new(StringArray::from_iter((0..len).map(|_| {
            let r = next();
            (r % 50 != 0).then(|| LABELS[(r % LABELS.len() as u64) as usize])
        })));
        let prices: ArrayRef = Arc::new(Float64Array::from_iter_values(
            (0..len).map(|_| (next() % 1_000_000) as f64 / 100.0),
        ));
        let cities: ArrayRef = Arc::new(StringArray::from_iter_values(
            (0..len).map(|_| format!("city-{:04}", next() % 2000)),
        ));
        // Days from 2020-01-01 (18262) over six years, some missing.
        let days: ArrayRef = Arc::new(Date32Array::from_iter((0..len).map(|_| {
            let r = next();
            (r % 40 != 0).then(|| 18_262 + (r % 2_192) as i32)
        })));
        // Timestamps in UTC across 2024, weighted towards working hours.
        let times: ArrayRef = Arc::new(
            TimestampMicrosecondArray::from_iter_values((0..len).map(|_| {
                let r = next();
                let day = (r % 366) as i64;
                let hour = [9, 10, 11, 13, 14, 15, 16, 20, 2][(r / 366 % 9) as usize];
                let second = (r / 3_294 % 3_600) as i64;
                (1_704_067_200 + day * 86_400 + hour * 3_600 + second) * 1_000_000
            }))
            .with_timezone("UTC"),
        );
        let batch = RecordBatch::try_from_iter(vec![
            ("id", ids),
            ("label", labels),
            ("price", prices),
            ("city", cities),
            ("day", days),
            ("updated", times),
        ])
        .unwrap();
        let w = writer.get_or_insert_with(|| {
            ArrowWriter::try_new(file.take().unwrap(), batch.schema(), Some(props.clone())).unwrap()
        });
        w.write(&batch).unwrap();
    }
    writer.unwrap().close().unwrap();
    println!("wrote {n} rows to {path}");
}
