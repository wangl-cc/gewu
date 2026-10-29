//! Write a synthetic Parquet file for manual and performance testing.
//!
//!     cargo run --release -p gewu-core --example gen_sample -- out.parquet 5000000

mod sample;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "sample.parquet".into());
    let n: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(1_000_000);
    sample::write(path.as_ref(), n);
    println!("wrote {n} rows to {path}");
}
