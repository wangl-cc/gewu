//! Export an immutable view snapshot using bounded working buffers. An export
//! never changes its document and only replaces its destination after success.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use arrow::{
    array::{Array, ArrayRef, RecordBatch, UInt32Array},
    compute::take,
    datatypes::{FieldRef, SchemaRef},
    error::ArrowError,
    json::writer::{make_encoder, Encoder, EncoderFactory, EncoderOptions, NullableEncoder},
    util::display::{ArrayFormatter, FormatOptions},
};
use parquet::{arrow::ArrowWriter, basic::Compression, file::properties::WriterProperties};

use crate::{Error, Result};

const CHUNK_ROWS: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Tsv,
    Parquet,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ExportOptions {
    /// Encode nested CSV/TSV cells as JSON. Otherwise nested columns are
    /// rejected. Parquet always preserves nested types and ignores this flag.
    pub convert_nested_to_json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    /// Rows processed so far. Reaching `total_rows` does not imply the final
    /// file is committed; wait for `write` to return successfully.
    pub rows_written: u64,
    pub total_rows: u64,
}

/// A single-use export with its own cancellation and progress. The snapshot
/// shares immutable Arrow buffers and row indices with its source document.
pub struct Export {
    source: PathBuf,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    rows: Arc<Vec<u32>>,
    format: ExportFormat,
    started: AtomicBool,
    cancelled: AtomicBool,
    rows_written: AtomicU64,
    /// Serializes cancellation with the final rename. Cancellation after that
    /// commit is a no-op; before it, the destination is left untouched.
    committed: Mutex<bool>,
}

impl Export {
    pub(crate) fn new(
        source: PathBuf,
        schema: SchemaRef,
        columns: Vec<ArrayRef>,
        rows: Arc<Vec<u32>>,
        format: ExportFormat,
    ) -> Self {
        Self {
            source,
            schema,
            columns,
            rows,
            format,
            started: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            rows_written: AtomicU64::new(0),
            committed: Mutex::new(false),
        }
    }

    pub fn progress(&self) -> ExportProgress {
        ExportProgress {
            rows_written: self.rows_written.load(Ordering::Relaxed),
            total_rows: self.rows.len() as u64,
        }
    }

    pub fn cancel(&self) {
        let committed = self.committed.lock().unwrap();
        if !*committed {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Write on a background thread. Headers are included for CSV/TSV, nulls
    /// are empty fields, and scalar values use Arrow's full display precision.
    /// Nested values are compact JSON, with explicit null children. Non-finite
    /// floats inside JSON are strings ("NaN", "inf", "-inf"), not nulls.
    /// Parquet preserves Arrow types and nulls. A sibling temporary file is
    /// atomically renamed only after the complete output has been flushed.
    ///
    /// The source file (including aliases), an invalid destination, reuse of
    /// this job, formatting failures, and cancellation return errors without
    /// replacing the destination. A failed job cannot be retried.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(Error::InvalidArgument(
                "this export has already been started".into(),
            ));
        }
        self.check_cancelled()?;
        let path = path.as_ref();
        self.check_destination(path)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".gewu-export-")
            .tempfile_in(parent)?;
        match self.format {
            ExportFormat::Csv => self.write_delimited(temporary.as_file_mut(), b',')?,
            ExportFormat::Tsv => self.write_delimited(temporary.as_file_mut(), b'\t')?,
            ExportFormat::Parquet => self.write_parquet(temporary.as_file_mut())?,
        }
        self.check_cancelled()?;
        temporary.as_file().sync_all()?;
        let mut committed = self.committed.lock().unwrap();
        self.check_cancelled()?;
        self.check_destination(path)?;
        temporary.persist(path).map_err(|error| error.error)?;
        *committed = true;
        Ok(())
    }

    fn check_destination(&self, path: &Path) -> Result<()> {
        let same_path = match (self.source.canonicalize(), path.canonicalize()) {
            (Ok(source), Ok(destination)) => source == destination,
            _ => false,
        };
        // Hard links have different canonical paths but share the same file.
        #[cfg(unix)]
        let same_file = {
            use std::os::unix::fs::MetadataExt;
            match (std::fs::metadata(&self.source), std::fs::metadata(path)) {
                (Ok(source), Ok(destination)) => {
                    source.dev() == destination.dev() && source.ino() == destination.ino()
                }
                _ => false,
            }
        };
        #[cfg(not(unix))]
        let same_file = false;
        if same_path || same_file {
            return Err(Error::InvalidArgument(
                "choose a different path from the open source file".into(),
            ));
        }
        Ok(())
    }

    fn write_delimited(&self, file: impl Write, delimiter: u8) -> Result<()> {
        let mut writer = BufWriter::new(file);
        for (i, field) in self.schema.fields().iter().enumerate() {
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
        let mut formatters = self
            .columns
            .iter()
            .zip(self.schema.fields())
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
        let nulls = self
            .columns
            .iter()
            .map(|array| array.logical_nulls())
            .collect::<Vec<_>>();
        let mut cell = String::new();
        let mut json_cell = Vec::new();
        for chunk in self.rows.chunks(CHUNK_ROWS) {
            self.check_cancelled()?;
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
            self.rows_written
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        writer.flush()?;
        self.check_cancelled()
    }

    fn write_parquet(&self, file: &mut File) -> Result<()> {
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_max_row_group_row_count(Some(64 * 1024))
            .set_max_row_group_bytes(Some(16 * 1024 * 1024))
            .build();
        let mut writer = ArrowWriter::try_new(file, self.schema.clone(), Some(properties))?;
        for chunk in self.rows.chunks(CHUNK_ROWS) {
            self.check_cancelled()?;
            let indices = UInt32Array::from(chunk.to_vec());
            let columns = self
                .columns
                .iter()
                .map(|array| take(array.as_ref(), &indices, None))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let batch = RecordBatch::try_new(self.schema.clone(), columns)?;
            writer.write(&batch)?;
            self.rows_written
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        self.check_cancelled()?;
        writer.close()?;
        Ok(())
    }
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
    use arrow::{
        array::{DictionaryArray, Int64Array, Int8Array, NullArray, StringArray},
        datatypes::{DataType, Field, Int8Type, Schema},
    };

    use super::*;

    #[test]
    fn cancellation_during_a_write_stops_before_processing_every_chunk() {
        let rows = CHUNK_ROWS * 3;
        let export = Export::new(
            PathBuf::from("source.parquet"),
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from_iter_values(0..rows as i64))],
            Arc::new((0..rows as u32).collect()),
            ExportFormat::Csv,
        );
        struct CancellingWriter<'a>(&'a Export);
        impl Write for CancellingWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.cancel();
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(
            export.write_delimited(CancellingWriter(&export), b','),
            Err(Error::Cancelled)
        ));
        assert!(export.progress().rows_written < export.progress().total_rows);
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
        let export = Export::new(
            PathBuf::from("source.parquet"),
            batch.schema(),
            batch.columns().to_vec(),
            Arc::new(vec![0, 1]),
            ExportFormat::Csv,
        );
        let mut bytes = Vec::new();
        export.write_delimited(&mut bytes, b',').unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "dictionary,null\r\n,\r\n\"\",\r\n"
        );
    }
}
