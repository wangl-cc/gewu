//! CSV and TSV decoding and encoding through Arrow's CSV adapter.

use std::{
    fs::File,
    io::{Seek, Write},
    sync::Arc,
};

use arrow::{
    array::RecordBatch,
    datatypes::{DataType, Schema},
};

use super::{collect_batches, ExportData, LoadedFile, WriteProgress, CHUNK_ROWS};
use crate::{FileMetadata, OpenOptions, Result};

pub(super) fn read(mut file: File, delimiter: u8, options: &OpenOptions) -> Result<LoadedFile> {
    let format = arrow_csv::reader::Format::default()
        .with_header(options.has_header)
        .with_delimiter(delimiter);
    // Scan the complete input for a stable schema: later mixed values must
    // not turn into nulls or fail solely because they were outside a sample.
    let (schema, _) = format.infer_schema(&mut file, None)?;
    file.rewind()?;
    let schema = preserve_integer_precision(&mut file, schema, delimiter, options.has_header)?;
    let schema = Arc::new(schema);
    let reader = arrow_csv::ReaderBuilder::new(schema.clone())
        .with_format(format)
        .with_batch_size(64 * 1024)
        .build(file)?;
    Ok(LoadedFile {
        batch: collect_batches(&schema, reader)?,
        metadata: FileMetadata::Delimited,
    })
}

/// Arrow promotes a mixture of integer and fractional literals to Float64.
/// Retain that column as text if this would change an integer's value. Only
/// files with inferred float columns need this extra scan, using csv's parser
/// so quoted records and embedded newlines obey the same syntax as Arrow.
fn preserve_integer_precision(
    file: &mut File,
    schema: Schema,
    delimiter: u8,
    has_header: bool,
) -> Result<Schema> {
    let floating: Vec<usize> = schema
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(i, field)| (field.data_type() == &DataType::Float64).then_some(i))
        .collect();
    if floating.is_empty() {
        return Ok(schema);
    }
    let mut preserve = vec![false; schema.fields().len()];
    {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(has_header)
            .from_reader(&mut *file);
        let mut record = csv::StringRecord::new();
        while reader.read_record(&mut record)? {
            for &column in &floating {
                if let Some(value) = record.get(column).and_then(|s| s.parse::<i64>().ok()) {
                    preserve[column] |= (value as f64) as i128 != i128::from(value);
                }
            }
        }
    }
    file.rewind()?;
    let fields = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| {
            if preserve[i] {
                Arc::new(field.as_ref().clone().with_data_type(DataType::Utf8))
            } else {
                field.clone()
            }
        })
        .collect::<Vec<_>>();
    Ok(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

pub(super) fn write(
    file: impl Write,
    delimiter: u8,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<()> {
    progress.check()?;
    let mut writer = arrow_csv::WriterBuilder::new()
        .with_delimiter(delimiter)
        .with_line_terminator(arrow_csv::writer::Terminator::CRLF)
        .build(file);
    // Emit headers even when the snapshot has no rows; this also validates
    // the schema before any row processing.
    writer.write(&RecordBatch::new_empty(data.schema.clone()))?;
    for rows in data.rows.chunks(CHUNK_ROWS) {
        progress.check()?;
        writer.write(&data.batch(rows)?)?;
        progress.advance(rows.len());
    }
    progress.check()
}

#[cfg(test)]
mod tests {
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
            schema: &schema,
            columns: batch.columns(),
            rows: &rows,
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
            write(CancellingWriter(&cancelled), b',', data, progress),
            Err(crate::Error::Cancelled)
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
            schema: &schema,
            columns: batch.columns(),
            rows: &[0, 1],
        };
        let progress = WriteProgress {
            cancelled: &cancelled,
            rows_written: &written,
        };
        let mut bytes = Vec::new();
        write(&mut bytes, b',', data, progress).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "dictionary,null\r\n,\r\n,\r\n"
        );
    }
}
