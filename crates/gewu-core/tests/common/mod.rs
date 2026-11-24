use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use gewu_core::Dataset;
use parquet::arrow::ArrowWriter;

pub fn sample() -> Arc<Dataset> {
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
    open_batch(&batch)
}

pub fn open_batch(batch: &RecordBatch) -> Arc<Dataset> {
    let file = tempfile::Builder::new()
        .suffix(".parquet")
        .tempfile()
        .unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(batch).unwrap();
    writer.close().unwrap();
    Arc::new(Dataset::open(file.path()).unwrap())
}
