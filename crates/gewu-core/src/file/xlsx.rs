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
use crate::{FileMetadata, OpenOptions, Result};

fn worksheet_names(workbook: &Xlsx<File>) -> Vec<String> {
    workbook
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect()
}

pub(super) fn sheet_names(file: File) -> Result<Vec<String>, crate::FileError> {
    Ok(worksheet_names(&Xlsx::new(file)?))
}

pub(super) fn read(file: File, options: &OpenOptions) -> Result<LoadedFile, crate::FileError> {
    let mut workbook = Xlsx::new(file)?;
    let sheets = worksheet_names(&workbook);
    let sheet = options
        .sheet
        .as_ref()
        .or_else(|| sheets.first())
        .ok_or(crate::FileError::NoWorksheets)?
        .clone();
    if !sheets.contains(&sheet) {
        return Err(crate::FileError::SheetNotFound { sheet });
    }
    let range = workbook.worksheet_range(&sheet)?;
    // Excel error values are data errors, including errors used as headers.
    // Do not silently turn them into nulls or ordinary text.
    let origin = range.start().unwrap_or((0, 0));
    for (row, values) in range.rows().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if let Data::Error(error) = value {
                return Err(crate::FileError::ErrorCell {
                    sheet: sheet.clone(),
                    row: origin.0 + row as u32 + 1,
                    column: origin.1 + column as u32 + 1,
                    error: error.to_string(),
                });
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
) -> Result<(), crate::ExportError> {
    if data.rows.len() > 1_048_575 {
        return Err(crate::ExportError::XlsxRows {
            actual: data.rows.len() as u64,
            limit: 1_048_575,
        });
    }
    if data.columns.len() > 16_384 {
        return Err(crate::ExportError::XlsxColumns {
            actual: data.columns.len() as u32,
            limit: 16_384,
        });
    }
    if data
        .columns
        .iter()
        .zip(data.as_text)
        .any(|(column, &as_text)| !as_text && column.data_type().is_nested())
    {
        return Err(crate::InternalError::Invariant(
            "nested columns reached the XLSX writer despite export validation".into(),
        )
        .into());
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
        for row in chunk.iter() {
            for (column, array) in data.columns.iter().enumerate() {
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
#[path = "xlsx_tests.rs"]
mod tests;
