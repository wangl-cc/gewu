use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use arrow::{
    array::{
        ArrayRef, DictionaryArray, Int64Array, Int8Array, NullArray, RecordBatch, StringArray,
    },
    datatypes::Int8Type,
};

use super::*;

#[test]
fn cancellation_during_a_write_stops_before_processing_every_chunk() {
    let rows = CHUNK_ROWS * 3;
    let batch = RecordBatch::try_from_iter(vec![(
        "id",
        Arc::new(Int64Array::from_iter_values(0..rows as i64)) as ArrayRef,
    )])
    .unwrap();
    let schema = batch.schema();
    let rows: Vec<u32> = (0..rows as u32).collect();
    let cancelled = AtomicBool::new(false);
    let written = AtomicU64::new(0);
    let data = ExportData {
        schema: schema.clone(),
        columns: batch.columns(),
        rows: crate::row_ids::RowSlice::Selected(&rows),
        as_text: &[false; 8][..batch.num_columns()],
    };
    let progress = WriteProgress {
        cancelled: &cancelled,
        rows_written: &written,
    };
    struct CancellingWriter<'a>(&'a AtomicBool);
    impl Write for CancellingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            // Let header emission finish, then cancel while row data is
            // being written so this exercises the chunk boundary.
            if bytes != b"id\r\n" {
                self.0.store(true, Ordering::Relaxed);
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        write(CancellingWriter(&cancelled), b',', true, data, progress),
        Err(crate::ExportError::Cancelled)
    ));
    assert!(written.load(Ordering::Relaxed) > 0);
    assert!(written.load(Ordering::Relaxed) < rows.len() as u64);
}

#[test]
fn logical_nulls_export_as_empty_fields() {
    let dictionary = Arc::new(DictionaryArray::<Int8Type>::new(
        Int8Array::from(vec![0, 1]),
        Arc::new(StringArray::from(vec![None, Some("")])),
    ));
    let batch = RecordBatch::try_from_iter(vec![
        ("dictionary", dictionary as ArrayRef),
        ("null", Arc::new(NullArray::new(2)) as ArrayRef),
    ])
    .unwrap();
    let schema = batch.schema();
    let cancelled = AtomicBool::new(false);
    let written = AtomicU64::new(0);
    let data = ExportData {
        schema: schema.clone(),
        columns: batch.columns(),
        rows: crate::row_ids::RowSlice::Selected(&[0, 1]),
        as_text: &[false; 8][..batch.num_columns()],
    };
    let progress = WriteProgress {
        cancelled: &cancelled,
        rows_written: &written,
    };
    let mut bytes = Vec::new();
    write(&mut bytes, b',', true, data, progress).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "dictionary,null\r\n,\r\n,\r\n"
    );
}
