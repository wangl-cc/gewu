use std::{fs, sync::Arc};

use arrow::{
    array::{
        Array, ArrayRef, Date32Array, Float64Array, Float64Builder, Int64Array, Int64Builder,
        ListArray, ListBuilder, MapBuilder, RecordBatch, StringArray, StringBuilder, StructArray,
    },
    buffer::{NullBuffer, OffsetBuffer, ScalarBuffer},
    datatypes::{DataType, Field, Schema},
};
use gewu_core::{Bound, Document, Error, ExportFormat, ExportOptions, Filter, SortKey, ViewState};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use tempfile::TempDir;

fn sample() -> (TempDir, Document) {
    let directory = tempfile::tempdir().unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("text,\"name", DataType::Utf8, true),
        Field::new("value", DataType::Float64, true),
        Field::new("date", DataType::Date32, true),
    ]));
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![3, 1, 4, 2, 9_007_199_254_740_993])),
        Arc::new(StringArray::from(vec![
            Some("a,\"雪\"\nline"),
            Some("omit"),
            Some(""),
            Some("tab\tcell\rtail"),
            None,
        ])),
        Arc::new(Float64Array::from(vec![
            Some(1.2345678901234567),
            Some(1.0),
            None,
            Some(-0.0),
            Some(f64::MIN_POSITIVE),
        ])),
        Arc::new(Date32Array::from(vec![
            Some(0),
            None,
            Some(1),
            Some(2),
            Some(-1),
        ])),
    ];
    let batch = RecordBatch::try_new(schema, arrays).unwrap();
    let path = directory.path().join("source.parquet");
    let mut writer =
        ArrowWriter::try_new(fs::File::create(&path).unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    (directory, Document::open(&path).unwrap())
}

fn document_from_batch(batch: &RecordBatch) -> (TempDir, Document) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested.parquet");
    let mut writer =
        ArrowWriter::try_new(fs::File::create(&path).unwrap(), batch.schema(), None).unwrap();
    writer.write(batch).unwrap();
    writer.close().unwrap();
    (directory, Document::open(path).unwrap())
}

fn nested_sample() -> RecordBatch {
    let mut tags = ListBuilder::new(StringBuilder::new());
    tags.values().append_value("a,\"雪\"\t\n\\tail");
    tags.values().append_null();
    tags.values().append_value("");
    tags.append(true);
    tags.append(true);
    tags.append(false);
    tags.values().append_value("filtered out");
    tags.append(true);
    let tags = Arc::new(tags.finish());
    let profile = Arc::new(StructArray::new(
        vec![
            Field::new("na\"me\n", DataType::Utf8, true),
            Field::new("number", DataType::Int64, true),
            Field::new("tags", tags.data_type().clone(), true),
        ]
        .into(),
        vec![
            Arc::new(StringArray::from(vec![
                Some("Alice\r\n雪"),
                Some(""),
                None,
                Some("omit"),
            ])),
            Arc::new(Int64Array::from(vec![
                Some(9_007_199_254_740_993),
                None,
                None,
                Some(9),
            ])),
            tags.clone(),
        ],
        Some(NullBuffer::from(vec![true, true, false, true])),
    ));
    let items = Arc::new(ListArray::new(
        Arc::new(Field::new("item", profile.data_type().clone(), true)),
        OffsetBuffer::new(ScalarBuffer::from(vec![0, 1, 2, 3, 4])),
        profile.clone(),
        None,
    ));
    RecordBatch::try_from_iter(vec![
        (
            "id",
            Arc::new(Int64Array::from(vec![3, 1, 2, 9])) as ArrayRef,
        ),
        ("tags", tags as ArrayRef),
        ("profile", profile as ArrayRef),
        ("items", items as ArrayRef),
    ])
    .unwrap()
}

#[test]
fn nested_delimited_cells_are_parseable_json_in_snapshot_order() {
    use serde_json::{json, Value};

    let (directory, doc) = document_from_batch(&nested_sample());
    let version = doc
        .apply_view(&ViewState {
            sort: vec![SortKey {
                column: 0,
                descending: true,
            }],
            filters: vec![Filter::Range {
                column: 0,
                min: None,
                max: Some(Bound::Int(3)),
            }],
            search: None,
        })
        .unwrap()
        .version;
    let first = json!({"na\"me\n": "Alice\r\n雪", "number": 9_007_199_254_740_993i64,
        "tags": ["a,\"雪\"\t\n\\tail", null, ""]});
    let last = json!({"na\"me\n": "", "number": null, "tags": []});
    for (format, delimiter) in [(ExportFormat::Csv, b','), (ExportFormat::Tsv, b'\t')] {
        let path = directory.path().join(format!("nested.{format:?}"));
        let export = doc
            .prepare_export(version, &[3, 2, 1, 0], format, ExportOptions {
                convert_nested_to_json: true,
            })
            .unwrap();
        export.write(&path).unwrap();
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .from_path(path)
            .unwrap();
        assert_eq!(reader.headers().unwrap().iter().collect::<Vec<_>>(), [
            "items", "profile", "tags", "id"
        ]);
        let rows = reader.records().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|row| &row[3]).collect::<Vec<_>>(), [
            "3", "2", "1"
        ]);
        assert_eq!(
            serde_json::from_str::<Value>(&rows[0][0]).unwrap(),
            json!([first.clone()])
        );
        assert_eq!(serde_json::from_str::<Value>(&rows[0][1]).unwrap(), first);
        assert_eq!(
            serde_json::from_str::<Value>(&rows[0][2]).unwrap(),
            first["tags"]
        );
        assert_eq!(
            serde_json::from_str::<Value>(&rows[1][0]).unwrap(),
            json!([null])
        );
        assert_eq!(&rows[1][1], "");
        assert_eq!(&rows[1][2], "");
        assert_eq!(
            serde_json::from_str::<Value>(&rows[2][0]).unwrap(),
            json!([last.clone()])
        );
        assert_eq!(serde_json::from_str::<Value>(&rows[2][1]).unwrap(), last);
        assert_eq!(
            serde_json::from_str::<Value>(&rows[2][2]).unwrap(),
            json!([])
        );
        assert_eq!(export.progress().rows_written, 3);
    }
}

#[test]
fn nested_conversion_requires_opt_in_but_parquet_preserves_native_values() {
    let batch = nested_sample();
    let (directory, doc) = document_from_batch(&batch);
    let version = doc.current_view().version;
    assert_eq!(doc.nested_export_columns(&[3, 0, 1]).unwrap(), [
        "items", "tags"
    ]);
    assert!(doc.nested_export_columns(&[99]).is_err());
    for format in [ExportFormat::Csv, ExportFormat::Tsv] {
        let result = doc.prepare_export(version, &[0, 2], format, ExportOptions::default());
        assert!(matches!(result, Err(Error::Unsupported(message))
            if message.contains("profile") && message.contains("Parquet")));
        assert!(doc
            .prepare_export(version, &[0], format, ExportOptions::default())
            .is_ok());
    }
    let path = directory.path().join("native.parquet");
    doc.prepare_export(
        version,
        &[0, 1, 2, 3],
        ExportFormat::Parquet,
        ExportOptions::default(),
    )
    .unwrap()
    .write(&path)
    .unwrap();
    let result = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(result, batch);
}

#[test]
fn nested_json_preserves_non_finite_values_separately_from_nulls() {
    use serde_json::{json, Value};

    let mut values = ListBuilder::new(Float64Builder::new());
    values
        .values()
        .append_slice(&[f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 1.5]);
    values.values().append_null();
    values.append(true);
    let batch = RecordBatch::try_from_iter(vec![("values", Arc::new(values.finish()) as ArrayRef)])
        .unwrap();
    let (directory, doc) = document_from_batch(&batch);
    let path = directory.path().join("floats.csv");
    doc.prepare_export(
        doc.current_view().version,
        &[0],
        ExportFormat::Csv,
        ExportOptions {
            convert_nested_to_json: true,
        },
    )
    .unwrap()
    .write(&path)
    .unwrap();
    let row = csv::Reader::from_path(path)
        .unwrap()
        .records()
        .next()
        .unwrap()
        .unwrap();
    let value = serde_json::from_str::<Value>(&row[0]).unwrap();
    assert_eq!(value, json!(["NaN", "inf", "-inf", -0.0, 1.5, null]));
    assert_eq!(value[3].as_f64().unwrap().to_bits(), (-0.0f64).to_bits());
}

#[test]
fn unsupported_json_conversion_preserves_the_existing_destination() {
    let mut values = MapBuilder::new(None, Int64Builder::new(), StringBuilder::new());
    values.keys().append_value(1);
    values.values().append_value("value");
    values.append(true).unwrap();
    let batch =
        RecordBatch::try_from_iter(vec![("map", Arc::new(values.finish()) as ArrayRef)]).unwrap();
    let (directory, doc) = document_from_batch(&batch);
    let path = directory.path().join("map.csv");
    fs::write(&path, "existing data").unwrap();
    let export = doc
        .prepare_export(
            doc.current_view().version,
            &[0],
            ExportFormat::Csv,
            ExportOptions {
                convert_nested_to_json: true,
            },
        )
        .unwrap();
    assert!(matches!(export.write(&path), Err(Error::Arrow(_))));
    assert_eq!(fs::read_to_string(path).unwrap(), "existing data");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn nested_date_formatting_failure_does_not_become_json_data() {
    let dates = Arc::new(StructArray::from(vec![(
        Arc::new(Field::new("date", DataType::Date32, false)),
        Arc::new(Date32Array::from(vec![0, i32::MAX])) as ArrayRef,
    )]));
    let batch = RecordBatch::try_from_iter(vec![("dates", dates as ArrayRef)]).unwrap();
    let (directory, doc) = document_from_batch(&batch);
    let path = directory.path().join("dates.csv");
    fs::write(&path, "existing data").unwrap();
    let export = doc
        .prepare_export(
            doc.current_view().version,
            &[0],
            ExportFormat::Csv,
            ExportOptions {
                convert_nested_to_json: true,
            },
        )
        .unwrap();
    assert!(matches!(export.write(&path), Err(Error::Arrow(_))));
    assert_eq!(fs::read_to_string(path).unwrap(), "existing data");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

fn filtered(doc: &Document) -> u64 {
    doc.apply_view(&ViewState {
        sort: vec![SortKey {
            column: 0,
            descending: true,
        }],
        filters: vec![Filter::Range {
            column: 0,
            min: Some(Bound::Int(2)),
            max: None,
        }],
        search: None,
    })
    .unwrap()
    .version
}

#[test]
fn delimited_export_uses_snapshot_row_order_column_order_and_exact_values() {
    let (directory, doc) = sample();
    let version = filtered(&doc);
    for (format, expected) in [
        (
            ExportFormat::Csv,
            concat!(
                "\"text,\"\"name\",id,value,date\r\n",
                ",9007199254740993,2.2250738585072014e-308,1969-12-31\r\n",
                "\"\",4,,1970-01-02\r\n",
                "\"a,\"\"雪\"\"\nline\",3,1.2345678901234567,1970-01-01\r\n",
                "\"tab\tcell\rtail\",2,-0.0,1970-01-03\r\n",
            ),
        ),
        (
            ExportFormat::Tsv,
            concat!(
                "\"text,\"\"name\"\tid\tvalue\tdate\r\n",
                "\t9007199254740993\t2.2250738585072014e-308\t1969-12-31\r\n",
                "\"\"\t4\t\t1970-01-02\r\n",
                "\"a,\"\"雪\"\"\nline\"\t3\t1.2345678901234567\t1970-01-01\r\n",
                "\"tab\tcell\rtail\"\t2\t-0.0\t1970-01-03\r\n",
            ),
        ),
    ] {
        let path = directory.path().join(format!("result.{format:?}"));
        let export = doc
            .prepare_export(version, &[1, 0, 2, 3], format, ExportOptions::default())
            .unwrap();
        export.write(&path).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
        assert_eq!(export.progress().rows_written, 4);
        assert_eq!(export.progress().total_rows, 4);
    }
}

#[test]
fn parquet_roundtrip_preserves_types_nulls_and_captured_view() {
    let (directory, doc) = sample();
    let version = filtered(&doc);
    let export = doc
        .prepare_export(
            version,
            &[1, 0, 2, 3],
            ExportFormat::Parquet,
            ExportOptions::default(),
        )
        .unwrap();
    // Query changes and cancellation in the source do not alter the snapshot.
    doc.apply_view(&ViewState::default()).unwrap();
    doc.cancel_pending();
    let path = directory.path().join("result.parquet");
    export.write(&path).unwrap();
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap())
        .unwrap()
        .build()
        .unwrap();
    let batch = reader.next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 4);
    assert_eq!(
        batch.schema().as_ref(),
        &doc.schema().project(&[1, 0, 2, 3]).unwrap()
    );
    let ids = batch
        .column(1)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(ids.values().as_ref(), &[9_007_199_254_740_993, 4, 3, 2]);
    let strings = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(strings.is_null(0));
    assert_eq!(strings.value(1), "");
    assert_eq!(strings.value(2), "a,\"雪\"\nline");
    let floats = batch
        .column(2)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert_eq!(floats.value(0).to_bits(), f64::MIN_POSITIVE.to_bits());
    assert!(floats.is_null(1));
    assert_eq!(floats.value(2).to_bits(), 1.2345678901234567f64.to_bits());
    assert_eq!(floats.value(3).to_bits(), (-0.0f64).to_bits());
    let dates = batch
        .column(3)
        .as_any()
        .downcast_ref::<Date32Array>()
        .unwrap();
    assert_eq!(dates.values().as_ref(), &[-1, 1, 0, 2]);
    assert!(reader.next().is_none());
}

#[test]
fn empty_results_keep_headers_and_parquet_schema() {
    let (directory, doc) = sample();
    let version = doc
        .apply_view(&ViewState {
            search: Some("absent-value".into()),
            ..Default::default()
        })
        .unwrap()
        .version;
    for format in [ExportFormat::Csv, ExportFormat::Tsv, ExportFormat::Parquet] {
        let path = directory.path().join(format!("empty.{format:?}"));
        doc.prepare_export(version, &[0], format, ExportOptions::default())
            .unwrap()
            .write(&path)
            .unwrap();
        if format == ExportFormat::Parquet {
            let reader =
                ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap()).unwrap();
            assert_eq!(reader.metadata().file_metadata().num_rows(), 0);
            assert_eq!(reader.schema().field(0).data_type(), &DataType::Int64);
        } else {
            assert_eq!(fs::read_to_string(path).unwrap(), "id\r\n");
        }
    }
}

#[test]
fn stale_and_invalid_exports_are_rejected_before_writing() {
    let (_directory, doc) = sample();
    let old = doc.current_view().version;
    let version = filtered(&doc);
    assert!(matches!(
        doc.prepare_export(old, &[0], ExportFormat::Csv, ExportOptions::default()),
        Err(Error::StaleVersion { .. })
    ));
    for columns in [vec![], vec![0, 0], vec![99]] {
        assert!(matches!(
            doc.prepare_export(
                version,
                &columns,
                ExportFormat::Csv,
                ExportOptions::default()
            ),
            Err(Error::InvalidArgument(_))
        ));
    }
}

#[test]
fn cancellation_and_failures_preserve_existing_destination_and_remove_temporary_files() {
    let (directory, doc) = sample();
    let version = doc.current_view().version;
    let path = directory.path().join("existing.csv");
    fs::write(&path, "keep me").unwrap();
    let export = doc
        .prepare_export(version, &[0], ExportFormat::Csv, ExportOptions::default())
        .unwrap();
    export.cancel();
    assert!(matches!(export.write(&path), Err(Error::Cancelled)));
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
    assert!(matches!(
        export.write(&path),
        Err(Error::InvalidArgument(_))
    ));
    // A rename failure also discards the complete temporary output.
    let target_directory = directory.path().join("directory.csv");
    fs::create_dir(&target_directory).unwrap();
    let export = doc
        .prepare_export(version, &[0], ExportFormat::Csv, ExportOptions::default())
        .unwrap();
    assert!(matches!(export.write(&target_directory), Err(Error::Io(_))));
    assert!(target_directory.is_dir());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".gewu-export-")));
    // Successful export can replace a destination once, after all rows exist.
    let export = doc
        .prepare_export(version, &[0], ExportFormat::Csv, ExportOptions::default())
        .unwrap();
    export.write(&path).unwrap();
    assert!(fs::read_to_string(&path)
        .unwrap()
        .starts_with("id\r\n3\r\n1\r\n"));
    assert!(matches!(
        export.write(&path),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn formatting_failure_does_not_publish_a_partial_file() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("dates.parquet");
    let batch = RecordBatch::try_from_iter(vec![(
        "date",
        Arc::new(Date32Array::from(vec![0, i32::MAX])) as ArrayRef,
    )])
    .unwrap();
    let mut writer =
        ArrowWriter::try_new(fs::File::create(&source).unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let doc = Document::open(source).unwrap();
    let destination = directory.path().join("result.csv");
    fs::write(&destination, "existing data").unwrap();
    let export = doc
        .prepare_export(
            doc.current_view().version,
            &[0],
            ExportFormat::Csv,
            ExportOptions::default(),
        )
        .unwrap();
    assert!(matches!(export.write(&destination), Err(Error::Arrow(_))));
    assert_eq!(fs::read_to_string(destination).unwrap(), "existing data");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn source_file_and_aliases_cannot_be_overwritten() {
    let (directory, doc) = sample();
    let source = directory.path().join("source.parquet");
    let original = fs::read(&source).unwrap();
    let mut paths = vec![
        source.clone(),
        directory.path().join(".").join("source.parquet"),
    ];
    #[cfg(unix)]
    {
        let link = directory.path().join("link.parquet");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        paths.push(link);
        let hard_link = directory.path().join("hard-link.parquet");
        fs::hard_link(&source, &hard_link).unwrap();
        paths.push(hard_link);
    }
    for path in paths {
        let export = doc
            .prepare_export(
                doc.current_view().version,
                &[0],
                ExportFormat::Parquet,
                ExportOptions::default(),
            )
            .unwrap();
        assert!(matches!(export.write(path), Err(Error::InvalidArgument(_))));
    }
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn view_histogram_includes_the_columns_own_filter() {
    let (_directory, doc) = sample();
    let state = ViewState {
        filters: vec![Filter::Range {
            column: 0,
            min: Some(Bound::Int(2)),
            max: Some(Bound::Int(4)),
        }],
        ..Default::default()
    };
    let view = doc.apply_view(&state).unwrap();
    let filter_histogram = doc.histogram(0, Some(&state), 8).unwrap();
    let view_histogram = doc.view_histogram(view.version, 0, 8).unwrap();
    assert_eq!(filter_histogram.total, 5);
    assert_eq!(view_histogram.total, 3);
    assert_eq!(view_histogram.counts.iter().sum::<u64>(), 3);
    doc.apply_view(&ViewState::default()).unwrap();
    assert!(matches!(
        doc.view_histogram(view.version, 0, 8),
        Err(Error::StaleVersion { .. })
    ));
}
