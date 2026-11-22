//! CSV and TSV decoding and encoding through Arrow's CSV adapter.

use std::{
    fs::File,
    io::{Seek, Write},
    path::Path,
    sync::Arc,
};

use arrow::{
    array::RecordBatch,
    datatypes::{DataType, Schema},
};

use super::{collect_batches, ExportData, LoadedFile, WriteProgress, CHUNK_ROWS};
use crate::{FileMetadata, OpenOptions, Result};

pub(super) fn read(
    mut file: File,
    path: &Path,
    delimiter: u8,
    options: &OpenOptions,
) -> Result<LoadedFile, crate::FileError> {
    let format = arrow_csv::reader::Format::default()
        .with_header(options.has_header)
        .with_delimiter(delimiter);
    // Scan the complete input for a stable schema: later mixed values must
    // not turn into nulls or fail solely because they were outside a sample.
    let (schema, _) = format.infer_schema(&mut file, None)?;
    file.rewind()
        .map_err(|error| crate::IoError::new(crate::IoOperation::Seek, path, error))?;
    let schema =
        preserve_integer_precision(&mut file, path, schema, delimiter, options.has_header)?;
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
    path: &Path,
    schema: Schema,
    delimiter: u8,
    has_header: bool,
) -> Result<Schema, crate::FileError> {
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
    file.rewind()
        .map_err(|error| crate::IoError::new(crate::IoOperation::Seek, path, error))?;
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
    header: bool,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<(), crate::ExportError> {
    progress.check()?;
    let mut writer = arrow_csv::WriterBuilder::new()
        .with_delimiter(delimiter)
        .with_header(header)
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
#[path = "delimited_tests.rs"]
mod tests;
