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
    let fictitious = Data::DateTime(ExcelDateTime::new(60.0, ExcelDateTimeType::DateTime, false));
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
        Err(crate::FileError::SheetNotFound { .. })
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
            schema: batch.schema(),
            columns: batch.columns(),
            rows: crate::row_ids::RowSlice::Selected(&[1, 0]),
            as_text: &[false; 8][..batch.num_columns()],
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
        schema: schema.clone(),
        columns: batch.columns(),
        rows: crate::row_ids::RowSlice::Selected(&[]),
        as_text: &[false; 8][..batch.num_columns()],
    };
    let progress = WriteProgress {
        cancelled: &cancelled,
        rows_written: &rows_written,
    };
    write(file.as_file_mut(), data.clone(), progress).unwrap();
    let loaded = read(file.reopen().unwrap(), &OpenOptions::default()).unwrap();
    assert_eq!(loaded.batch.num_rows(), 0);
    assert_eq!(loaded.batch.schema().field(0).name(), "id");

    let excessive_rows = vec![0; 1_048_576];
    assert!(matches!(
        write(
            file.as_file_mut(),
            ExportData {
                rows: crate::row_ids::RowSlice::Selected(&excessive_rows),
                ..data.clone()
            },
            progress
        ),
        Err(crate::ExportError::XlsxRows { .. })
    ));
    let excessive_columns = vec![batch.column(0).clone(); 16_385];
    assert!(matches!(
        write(
            file.as_file_mut(),
            ExportData {
                columns: &excessive_columns,
                ..data.clone()
            },
            progress
        ),
        Err(crate::ExportError::XlsxColumns { .. })
    ));

    cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        write(file.as_file_mut(), data, progress),
        Err(crate::ExportError::Cancelled)
    ));
}
