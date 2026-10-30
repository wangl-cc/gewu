//! XLSX uses cached formula results; it never evaluates formulas. Reading uses
//! the worksheet's occupied range, so blank edges are not table rows/columns.
//! Exports use native booleans and numbers, but integers above Excel's 15-digit
//! precision, decimals, and temporal values are literal text to retain fidelity.

use std::{fs::File, sync::Arc};

use arrow::{
    array::{
        downcast_integer_array, Array, ArrayRef, AsArray, BooleanArray, DurationMillisecondArray,
        Float64Array, Int64Array, NullArray, RecordBatch, RecordBatchOptions, StringArray,
        TimestampMillisecondArray,
    },
    datatypes::{DataType, Field, Float16Type, Float32Type, Float64Type, Schema},
    util::display::{ArrayFormatter, FormatOptions},
};
use calamine::{Data, Reader, SheetType, Xlsx};
use rust_xlsxwriter::{Format, Workbook};

use super::{ExportData, LoadedFile, WriteProgress, CHUNK_ROWS};
use crate::{Error, FileMetadata, OpenOptions, Result};

fn worksheet_names(workbook: &Xlsx<File>) -> Vec<String> {
    workbook
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect()
}

pub(super) fn sheet_names(file: File) -> Result<Vec<String>> {
    Ok(worksheet_names(&Xlsx::new(file)?))
}

pub(super) fn read(file: File, options: &OpenOptions) -> Result<LoadedFile> {
    let mut workbook = Xlsx::new(file)?;
    let sheets = worksheet_names(&workbook);
    let sheet = options
        .sheet
        .as_ref()
        .or_else(|| sheets.first())
        .ok_or_else(|| Error::InvalidArgument("XLSX workbook contains no worksheets".into()))?
        .clone();
    if !sheets.contains(&sheet) {
        return Err(Error::InvalidArgument(format!(
            "XLSX worksheet {sheet:?} does not exist"
        )));
    }
    let range = workbook.worksheet_range(&sheet)?;
    // Excel error values are data errors, including errors used as headers.
    // Do not silently turn them into nulls or ordinary text.
    let origin = range.start().unwrap_or((0, 0));
    for (row, values) in range.rows().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if let Data::Error(error) = value {
                return Err(Error::InvalidArgument(format!(
                    "XLSX worksheet {sheet:?}, row {}, column {} contains {error}",
                    origin.0 as usize + row + 1,
                    origin.1 as usize + column + 1,
                )));
            }
        }
    }
    let skip = usize::from(options.has_header && range.height() > 0);
    let header = range.rows().next();
    let mut fields = Vec::with_capacity(range.width());
    let mut arrays = Vec::with_capacity(range.width());
    for column in 0..range.width() {
        let name = if options.has_header {
            header
                .map(|row| cell_text(&row[column]))
                .unwrap_or_default()
        } else {
            format!("column_{}", column + 1)
        };
        let values = range.rows().skip(skip).map(|row| &row[column]);
        let array = column_array(values);
        fields.push(Field::new(name, array.data_type().clone(), true));
        arrays.push(array);
    }
    Ok(LoadedFile {
        batch: RecordBatch::try_new_with_options(
            Arc::new(Schema::new(fields)),
            arrays,
            &RecordBatchOptions::new().with_row_count(Some(range.height() - skip)),
        )?,
        metadata: FileMetadata::Xlsx { sheet, sheets },
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColumnType {
    Null,
    Integer,
    Float,
    Boolean,
    Timestamp,
    Duration,
    Text,
}

fn column_array<'a>(values: impl Iterator<Item = &'a Data> + Clone) -> ArrayRef {
    let mut kind = ColumnType::Null;
    let mut large_integer = false;
    for value in values.clone() {
        let next = match value {
            Data::Empty => ColumnType::Null,
            Data::Int(value) => {
                large_integer |= !(-9_007_199_254_740_992..=9_007_199_254_740_992).contains(value);
                ColumnType::Integer
            }
            Data::Float(_) => ColumnType::Float,
            Data::Bool(_) => ColumnType::Boolean,
            Data::DateTime(value) if value.is_datetime() && timestamp(value).is_some() => {
                ColumnType::Timestamp
            }
            Data::DateTime(value) if value.is_duration() && duration(value).is_some() => {
                ColumnType::Duration
            }
            Data::DateTimeIso(value) if iso_timestamp(value).is_some() => ColumnType::Timestamp,
            _ => ColumnType::Text,
        };
        kind = match (kind, next) {
            (ColumnType::Null, next) => next,
            (kind, ColumnType::Null) => kind,
            (left, right) if left == right => left,
            (ColumnType::Integer, ColumnType::Float) | (ColumnType::Float, ColumnType::Integer) => {
                ColumnType::Float
            }
            _ => ColumnType::Text,
        };
    }
    // Mixing a large integer with floating point must not round that integer.
    if kind == ColumnType::Float && large_integer {
        kind = ColumnType::Text;
    }
    match kind {
        ColumnType::Null => Arc::new(NullArray::new(values.count())),
        ColumnType::Integer => Arc::new(Int64Array::from_iter(values.map(|value| match value {
            Data::Int(value) => Some(*value),
            _ => None,
        }))),
        ColumnType::Float => Arc::new(Float64Array::from_iter(values.map(|value| match value {
            Data::Int(value) => Some(*value as f64),
            Data::Float(value) => Some(*value),
            _ => None,
        }))),
        ColumnType::Boolean => Arc::new(BooleanArray::from_iter(values.map(|value| match value {
            Data::Bool(value) => Some(*value),
            _ => None,
        }))),
        ColumnType::Timestamp => Arc::new(TimestampMillisecondArray::from_iter(values.map(
            |value| match value {
                Data::DateTime(value) => timestamp(value),
                Data::DateTimeIso(value) => iso_timestamp(value),
                _ => None,
            },
        ))),
        ColumnType::Duration => Arc::new(DurationMillisecondArray::from_iter(values.map(
            |value| match value {
                Data::DateTime(value) => duration(value),
                _ => None,
            },
        ))),
        ColumnType::Text => {
            Arc::new(StringArray::from_iter(values.map(|value| {
                (!matches!(value, Data::Empty)).then(|| cell_text(value))
            })))
        }
    }
}

fn timestamp(value: &calamine::ExcelDateTime) -> Option<i64> {
    // Check before invoking chrono conversion: malformed huge serial values
    // must not saturate into a plausible timestamp or panic inside chrono.
    let days = value.as_f64();
    if !days.is_finite() || !(0.0..2_958_466.0).contains(&days) {
        return None;
    }
    let datetime = value.as_datetime()?;
    let (year, month, day, ..) = value.to_ymd_hms_milli();
    let date = chrono::NaiveDate::from_ymd_opt(year.into(), month.into(), day.into())?;
    // Serial day 60 in the 1900 epoch is Excel's fictitious February 29.
    // Also reject a conversion past the 1904 epoch's supported end date.
    (datetime.date() == date).then(|| datetime.and_utc().timestamp_millis())
}

fn duration(value: &calamine::ExcelDateTime) -> Option<i64> {
    let millis = value.as_f64() * 86_400_000.0;
    if millis.is_finite() && millis >= i64::MIN as f64 && millis < i64::MAX as f64 {
        Some(millis.round() as i64)
    } else {
        None
    }
}

fn iso_timestamp(value: &str) -> Option<i64> {
    let datetime = value
        .parse::<chrono::NaiveDateTime>()
        .or_else(|_| {
            value
                .parse::<chrono::NaiveDate>()
                .map(|date| date.and_time(chrono::NaiveTime::MIN))
        })
        .ok()?
        .and_utc();
    // Retain higher precision or timezone-bearing ISO strings as text instead
    // of discarding information in our timezone-free millisecond column.
    (datetime.timestamp_subsec_nanos() % 1_000_000 == 0).then(|| datetime.timestamp_millis())
}

fn cell_text(value: &Data) -> String {
    match value {
        Data::DateTime(value) if value.is_datetime() && timestamp(value).is_some() => {
            value.as_datetime().unwrap().to_string()
        }
        // Calamine's Display prints the original serial value for a duration
        // or an invalid date. ISO dates/durations retain their original text.
        _ => value.to_string(),
    }
}

pub(super) fn write(
    file: &mut File,
    data: ExportData<'_>,
    progress: WriteProgress<'_>,
) -> Result<()> {
    if data.rows.len() > 1_048_575 {
        return Err(Error::InvalidArgument(
            "XLSX supports at most 1048575 data rows plus a header".into(),
        ));
    }
    if data.columns.len() > 16_384 {
        return Err(Error::InvalidArgument(
            "XLSX supports at most 16384 columns".into(),
        ));
    }
    if data
        .columns
        .iter()
        .any(|column| column.data_type().is_nested())
    {
        return Err(Error::Unsupported(
            "XLSX cannot export nested columns".into(),
        ));
    }
    progress.check()?;
    let options = FormatOptions::default();
    let formatters = data
        .columns
        .iter()
        .map(|column| ArrayFormatter::try_new(column.as_ref(), &options))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let nulls = data
        .columns
        .iter()
        .map(|column| column.logical_nulls())
        .collect::<Vec<_>>();
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet_with_constant_memory()?;
    for (column, field) in data.schema.fields().iter().enumerate() {
        worksheet.write_string(0, column as u16, field.name())?;
    }
    let blank = Format::new();
    let mut text = String::new();
    let mut output_row = 1;
    for chunk in data.rows.chunks(CHUNK_ROWS) {
        progress.check()?;
        for &source_row in chunk {
            for (column, array) in data.columns.iter().enumerate() {
                let row = source_row as usize;
                let target_column = column as u16;
                if nulls[column]
                    .as_ref()
                    .is_some_and(|nulls| nulls.is_null(row))
                {
                    worksheet.write_blank(output_row, target_column, &blank)?;
                } else if array.data_type() == &DataType::Boolean {
                    worksheet.write_boolean(
                        output_row,
                        target_column,
                        array.as_boolean().value(row),
                    )?;
                } else if let Some(number) = excel_number(array.as_ref(), row) {
                    worksheet.write_number(output_row, target_column, number)?;
                } else {
                    text.clear();
                    formatters[column].value(row).write(&mut text)?;
                    worksheet.write_string(output_row, target_column, &text)?;
                }
            }
            output_row += 1;
        }
        progress.advance(chunk.len());
    }
    progress.check()?;
    workbook.save_to_writer(file)?;
    progress.check()
}

fn excel_number(array: &dyn Array, row: usize) -> Option<f64> {
    let number = match array.data_type() {
        DataType::Float16 => array.as_primitive::<Float16Type>().value(row).to_f64(),
        DataType::Float32 => f64::from(array.as_primitive::<Float32Type>().value(row)),
        DataType::Float64 => array.as_primitive::<Float64Type>().value(row),
        _ => {
            return downcast_integer_array!(array => {
                let value = array.value(row) as i128;
                (-999_999_999_999_999..=999_999_999_999_999).contains(&value)
                    .then_some(value as f64)
            }, _ => None);
        }
    };
    number.is_finite().then_some(number)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use arrow::array::{Decimal128Array, UInt64Array};
    use calamine::{ExcelDateTime, ExcelDateTimeType};

    use super::*;

    #[test]
    fn inference_uses_all_values_and_preserves_mixed_values() {
        let values = [Data::Empty, Data::Int(1), Data::Float(2.5)];
        let array = column_array(values.iter());
        assert_eq!(array.data_type(), &DataType::Float64);
        assert!(array.is_null(0));
        assert_eq!(array.as_primitive::<Float64Type>().value(2), 2.5);

        let values = [Data::Int(i64::MAX), Data::Float(2.5), Data::Empty];
        let array = column_array(values.iter());
        assert_eq!(array.as_string::<i32>().value(0), i64::MAX.to_string());
        assert!(array.is_null(2));

        let values = [Data::Bool(false), Data::String("later text".into())];
        let array = column_array(values.iter());
        assert_eq!(array.as_string::<i32>().value(0), "false");
    }

    #[test]
    fn excel_dates_and_durations_are_distinct_and_invalid_dates_remain_text() {
        let date = Data::DateTime(ExcelDateTime::new(
            45_943.5,
            ExcelDateTimeType::DateTime,
            false,
        ));
        let array = column_array([&date].into_iter());
        assert_eq!(
            array.data_type(),
            &DataType::Timestamp(arrow::datatypes::TimeUnit::Millisecond, None)
        );
        assert_eq!(cell_text(&date), "2025-10-13 12:00:00");
        let duration = Data::DateTime(ExcelDateTime::new(1.5, ExcelDateTimeType::TimeDelta, false));
        let array = column_array([&duration].into_iter());
        assert_eq!(
            array.data_type(),
            &DataType::Duration(arrow::datatypes::TimeUnit::Millisecond)
        );
        let invalid = Data::DateTime(ExcelDateTime::new(
            f64::MAX,
            ExcelDateTimeType::DateTime,
            false,
        ));
        assert_eq!(
            column_array([&invalid].into_iter()).data_type(),
            &DataType::Utf8
        );
        let fictitious =
            Data::DateTime(ExcelDateTime::new(60.0, ExcelDateTimeType::DateTime, false));
        assert_eq!(
            column_array([&fictitious].into_iter()).data_type(),
            &DataType::Utf8
        );
        let iso_date = Data::DateTimeIso("2025-10-13".into());
        assert_eq!(
            column_array([&iso_date].into_iter()).data_type(),
            &DataType::Timestamp(arrow::datatypes::TimeUnit::Millisecond, None)
        );
        let precise = Data::DateTimeIso("2025-10-13T12:00:00.123456".into());
        assert_eq!(
            column_array([&precise].into_iter()).data_type(),
            &DataType::Utf8
        );
    }

    fn save(workbook: &mut Workbook) -> tempfile::NamedTempFile {
        let mut file = tempfile::Builder::new().suffix(".xlsx").tempfile().unwrap();
        workbook.save_to_writer(file.as_file_mut()).unwrap();
        file
    }

    #[test]
    fn sheet_selection_headerless_and_empty_tables() {
        let mut workbook = Workbook::new();
        workbook.add_worksheet().set_name("empty").unwrap();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("data").unwrap();
        worksheet.set_hidden(true);
        worksheet.write_string(0, 0, "first").unwrap();
        worksheet.write_string(1, 0, "second").unwrap();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("header").unwrap();
        worksheet.write_string(0, 0, "id").unwrap();
        let file = save(&mut workbook);
        assert_eq!(crate::xlsx_sheet_names(file.path()).unwrap(), [
            "empty", "data", "header"
        ]);
        let empty = read(file.reopen().unwrap(), &OpenOptions::default()).unwrap();
        assert_eq!(empty.batch.num_rows(), 0);
        assert_eq!(empty.batch.num_columns(), 0);

        let options = OpenOptions {
            sheet: Some("data".into()),
            has_header: false,
            ..OpenOptions::default()
        };
        let loaded = read(file.reopen().unwrap(), &options).unwrap();
        assert_eq!(loaded.batch.num_rows(), 2);
        assert_eq!(loaded.batch.schema().field(0).name(), "column_1");
        assert_eq!(loaded.batch.column(0).as_string::<i32>().value(0), "first");
        assert_eq!(loaded.metadata, FileMetadata::Xlsx {
            sheet: "data".into(),
            sheets: vec!["empty".into(), "data".into(), "header".into()],
        });

        let options = OpenOptions {
            sheet: Some("header".into()),
            ..OpenOptions::default()
        };
        let loaded = read(file.reopen().unwrap(), &options).unwrap();
        assert_eq!(loaded.batch.num_rows(), 0);
        assert_eq!(loaded.batch.schema().field(0).name(), "id");

        let options = OpenOptions {
            sheet: Some("absent".into()),
            ..OpenOptions::default()
        };
        assert!(matches!(
            read(file.reopen().unwrap(), &options),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn export_preserves_selected_values_and_literal_strings() {
        let batch = RecordBatch::try_from_iter(vec![
            (
                "integer",
                Arc::new(UInt64Array::from(vec![42, u64::MAX])) as ArrayRef,
            ),
            (
                "decimal",
                Arc::new(
                    Decimal128Array::from(vec![123, 999999999999999999])
                        .with_precision_and_scale(20, 2)
                        .unwrap(),
                ) as ArrayRef,
            ),
            (
                "float",
                Arc::new(Float64Array::from(vec![f64::INFINITY, 1.25])) as ArrayRef,
            ),
            (
                "boolean",
                Arc::new(BooleanArray::from(vec![true, false])) as ArrayRef,
            ),
            (
                "literal",
                Arc::new(StringArray::from(vec![
                    "=SUM(A1:A2)",
                    "https://example.com",
                ])) as ArrayRef,
            ),
            ("null", Arc::new(NullArray::new(2)) as ArrayRef),
        ])
        .unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let cancelled = AtomicBool::new(false);
        let rows_written = AtomicU64::new(0);
        write(
            file.as_file_mut(),
            ExportData {
                schema: &batch.schema(),
                columns: batch.columns(),
                rows: &[1, 0],
            },
            WriteProgress {
                cancelled: &cancelled,
                rows_written: &rows_written,
            },
        )
        .unwrap();
        assert_eq!(rows_written.load(Ordering::Relaxed), 2);
        let mut workbook = Xlsx::new(file.reopen().unwrap()).unwrap();
        let range = workbook.worksheet_range_at(0).unwrap().unwrap();
        assert_eq!(range.get((1, 0)), Some(&Data::String(u64::MAX.to_string())));
        assert_eq!(range.get((2, 0)), Some(&Data::Float(42.0)));
        assert_eq!(
            range.get((1, 1)),
            Some(&Data::String("9999999999999999.99".into()))
        );
        assert_eq!(range.get((1, 2)), Some(&Data::Float(1.25)));
        assert_eq!(range.get((2, 2)), Some(&Data::String("inf".into())));
        assert_eq!(range.get((1, 3)), Some(&Data::Bool(false)));
        assert_eq!(range.get((2, 4)), Some(&Data::String("=SUM(A1:A2)".into())));
        assert_eq!(range.get((1, 5)), Some(&Data::Empty));
        assert!(workbook.worksheet_formula("Sheet1").unwrap().is_empty());
    }

    #[test]
    fn formulas_use_cached_values_and_excel_errors_are_reported() {
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_string(0, 0, "cached").unwrap();
        worksheet
            .write_formula(1, 0, rust_xlsxwriter::Formula::new("1+1").set_result("123"))
            .unwrap();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("error").unwrap();
        worksheet.write_string(0, 0, "error").unwrap();
        worksheet
            .write_formula(
                1,
                0,
                rust_xlsxwriter::Formula::new("1/0").set_result("#DIV/0!"),
            )
            .unwrap();
        workbook.worksheets_mut().swap(0, 1);
        let file = save(&mut workbook);
        // Even an unreadable first worksheet must not block selecting another.
        assert_eq!(crate::xlsx_sheet_names(file.path()).unwrap(), [
            "error", "Sheet1"
        ]);
        assert!(read(file.reopen().unwrap(), &OpenOptions::default()).is_err());
        let loaded = read(file.reopen().unwrap(), &OpenOptions {
            sheet: Some("Sheet1".into()),
            ..OpenOptions::default()
        })
        .unwrap();
        assert_eq!(
            loaded
                .batch
                .column(0)
                .as_primitive::<Float64Type>()
                .value(0),
            123.0
        );
        let options = OpenOptions {
            sheet: Some("error".into()),
            ..OpenOptions::default()
        };
        let error = read(file.reopen().unwrap(), &options)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("row 2, column 1 contains #DIV/0!"),
            "{error}"
        );
    }

    #[test]
    fn export_rejects_limits_and_cancellation_and_keeps_empty_headers() {
        let batch = RecordBatch::try_from_iter(vec![(
            "id",
            Arc::new(Int64Array::from(Vec::<i64>::new())) as ArrayRef,
        )])
        .unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let cancelled = AtomicBool::new(false);
        let rows_written = AtomicU64::new(0);
        let schema = batch.schema();
        let data = ExportData {
            schema: &schema,
            columns: batch.columns(),
            rows: &[],
        };
        let progress = WriteProgress {
            cancelled: &cancelled,
            rows_written: &rows_written,
        };
        write(file.as_file_mut(), data, progress).unwrap();
        let loaded = read(file.reopen().unwrap(), &OpenOptions::default()).unwrap();
        assert_eq!(loaded.batch.num_rows(), 0);
        assert_eq!(loaded.batch.schema().field(0).name(), "id");

        let excessive_rows = vec![0; 1_048_576];
        assert!(matches!(
            write(
                file.as_file_mut(),
                ExportData {
                    rows: &excessive_rows,
                    ..data
                },
                progress
            ),
            Err(Error::InvalidArgument(_))
        ));
        let excessive_columns = vec![batch.column(0).clone(); 16_385];
        assert!(matches!(
            write(
                file.as_file_mut(),
                ExportData {
                    columns: &excessive_columns,
                    ..data
                },
                progress
            ),
            Err(Error::InvalidArgument(_))
        ));

        cancelled.store(true, Ordering::Relaxed);
        assert!(matches!(
            write(file.as_file_mut(), data, progress),
            Err(Error::Cancelled)
        ));
    }
}
