//! CSV and TSV decoding and encoding. The writer preserves quoted empty strings
//! separately from null fields and the existing nested JSON representation.

use std::{
    fs::File,
    io::{BufWriter, Seek, Write},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::Array,
    datatypes::{DataType, FieldRef, Schema},
    error::ArrowError,
    json::writer::{make_encoder, Encoder, EncoderFactory, EncoderOptions, NullableEncoder},
    util::display::{ArrayFormatter, FormatOptions},
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
    let mut writer = BufWriter::new(file);
    for (i, field) in data.schema.fields().iter().enumerate() {
        if i > 0 {
            writer.write_all(&[delimiter])?;
        }
        write_field(&mut writer, field.name().as_bytes(), delimiter)?;
    }
    writer.write_all(b"\r\n")?;
    let options = FormatOptions::default();
    let json_error = Arc::new(OnceLock::new());
    let json_options = EncoderOptions::default()
        .with_explicit_nulls(true)
        .with_encoder_factory(Arc::new(ExportJsonEncoderFactory {
            error: json_error.clone(),
        }));
    let mut formatters = data
        .columns
        .iter()
        .zip(data.schema.fields())
        .map(|(array, field)| {
            if array.data_type().is_nested() {
                make_encoder(field, array.as_ref(), &json_options).map(CellFormatter::Json)
            } else {
                ArrayFormatter::try_new(array.as_ref(), &options).map(CellFormatter::Scalar)
            }
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Dictionary values and Null arrays can be logically null without a
    // physical validity bitmap on the outer array.
    let nulls = data
        .columns
        .iter()
        .map(|array| array.logical_nulls())
        .collect::<Vec<_>>();
    let mut cell = String::new();
    let mut json_cell = Vec::new();
    for chunk in data.rows.chunks(CHUNK_ROWS) {
        progress.check()?;
        for &row in chunk {
            for (column, (nulls, formatter)) in nulls.iter().zip(&mut formatters).enumerate() {
                if column > 0 {
                    writer.write_all(&[delimiter])?;
                }
                if !nulls
                    .as_ref()
                    .is_some_and(|nulls| nulls.is_null(row as usize))
                {
                    let bytes = match formatter {
                        CellFormatter::Scalar(formatter) => {
                            cell.clear();
                            formatter.value(row as usize).write(&mut cell)?;
                            cell.as_bytes()
                        }
                        CellFormatter::Json(encoder) => {
                            json_cell.clear();
                            encoder.encode(row as usize, &mut json_cell);
                            if let Some(error) = json_error.get() {
                                return Err(ArrowError::JsonError(error.clone()).into());
                            }
                            &json_cell
                        }
                    };
                    write_field(&mut writer, bytes, delimiter)?;
                }
            }
            writer.write_all(b"\r\n")?;
        }
        progress.advance(chunk.len());
    }
    writer.flush()?;
    progress.check()
}

enum CellFormatter<'a> {
    Scalar(ArrayFormatter<'a>),
    Json(NullableEncoder<'a>),
}

/// Keep non-finite floats distinct from null, and propagate temporal formatting
/// errors instead of encoding Arrow's diagnostic display text as data.
#[derive(Debug)]
struct ExportJsonEncoderFactory {
    // Arrow's Encoder callback cannot return an error. Record the first failure
    // and check it before writing the cell. Successful encodes only read this.
    error: Arc<OnceLock<String>>,
}

impl EncoderFactory for ExportJsonEncoderFactory {
    fn make_default_encoder<'a>(
        &self,
        _field: &'a FieldRef,
        array: &'a dyn Array,
        _options: &'a EncoderOptions,
    ) -> std::result::Result<Option<NullableEncoder<'a>>, ArrowError> {
        if !array.data_type().is_floating() && !array.data_type().is_temporal() {
            return Ok(None);
        }
        let formatter = ArrayFormatter::try_new(array, &FormatOptions::default())?;
        let encoder: Box<dyn Encoder + 'a> = if array.data_type().is_floating() {
            Box::new(JsonFloatEncoder(formatter))
        } else {
            Box::new(JsonTemporalEncoder {
                formatter,
                text: String::new(),
                error: self.error.clone(),
            })
        };
        Ok(Some(NullableEncoder::new(encoder, array.nulls().cloned())))
    }
}

struct JsonTemporalEncoder<'a> {
    formatter: ArrayFormatter<'a>,
    text: String,
    error: Arc<OnceLock<String>>,
}

impl Encoder for JsonTemporalEncoder<'_> {
    fn encode(&mut self, row: usize, out: &mut Vec<u8>) {
        self.text.clear();
        match self.formatter.value(row).write(&mut self.text) {
            Ok(()) => serde_json::to_writer(out, &self.text).expect("writing a string to a Vec"),
            Err(error) => {
                let _ = self.error.set(error.to_string());
            }
        }
    }
}

struct JsonFloatEncoder<'a>(ArrayFormatter<'a>);

impl Encoder for JsonFloatEncoder<'_> {
    fn encode(&mut self, row: usize, out: &mut Vec<u8>) {
        let start = out.len();
        // Float formatting and writing to a Vec cannot fail.
        write!(out, "{}", self.0.value(row)).expect("infallible float formatting");
        if matches!(&out[start..], b"NaN" | b"inf" | b"-inf") {
            out.insert(start, b'"');
            out.push(b'"');
        }
    }
}

fn write_field(writer: &mut impl Write, value: &[u8], delimiter: u8) -> std::io::Result<()> {
    if !value.is_empty()
        && !value
            .iter()
            .any(|byte| [delimiter, b'"', b'\r', b'\n'].contains(byte))
    {
        return writer.write_all(value);
    }
    writer.write_all(b"\"")?;
    for (i, part) in value.split(|&byte| byte == b'"').enumerate() {
        if i > 0 {
            writer.write_all(b"\"\"")?;
        }
        writer.write_all(part)?;
    }
    writer.write_all(b"\"")
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
                self.0.store(true, Ordering::Relaxed);
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
            "dictionary,null\r\n,\r\n\"\",\r\n"
        );
    }
}
