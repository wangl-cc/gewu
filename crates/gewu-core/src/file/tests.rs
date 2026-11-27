use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array};

use super::*;

#[test]
fn single_batch_reuses_its_columns_without_concatenating() {
    let values: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let batch = RecordBatch::try_from_iter([("value", values)]).unwrap();
    let original = batch.column(0).clone();
    let result = collect_batches(
        &batch.schema(),
        std::iter::once(Ok::<_, arrow::error::ArrowError>(batch)),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&original, result.column(0)));
}
