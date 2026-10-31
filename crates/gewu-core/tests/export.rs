#![cfg(all(feature = "parquet", feature = "csv"))]

use std::{fs, sync::Arc};

use arrow::{
    array::{
        Array, ArrayRef, Date32Array, Float64Array, Int64Array, ListArray, ListBuilder,
        RecordBatch, StringArray, StringBuilder, StructArray,
    },
    buffer::{NullBuffer, OffsetBuffer, ScalarBuffer},
    datatypes::{DataType, Field, Schema},
};
use gewu_core::{
    Document, Error, ExportOptions, FileType, Filter, ParquetCompression, SortKey, ViewState,
};
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
fn flat_formats_reject_nested_columns_but_parquet_preserves_native_values() {
    let batch = nested_sample();
    let (directory, doc) = document_from_batch(&batch);
    let version = doc.current_view().version;
    assert_eq!(doc.nested_export_columns(&[3, 0, 1]).unwrap(), [
        "items", "tags"
    ]);
    assert!(doc.nested_export_columns(&[99]).is_err());
    for format in [FileType::Csv, FileType::Tsv, FileType::Xlsx]
        .into_iter()
        .filter(|format| format.is_enabled())
    {
        let result = doc.prepare_export(version, &[0, 2], format);
        assert!(matches!(result, Err(Error::Unsupported(message))
            if message.contains("profile") && message.contains("Parquet")));
        assert!(matches!(
            doc.prepare_export(version, &[1], format),
            Err(Error::Unsupported(_))
        ));
        assert!(doc.prepare_export(version, &[0], format).is_ok());
    }
    let path = directory.path().join("native.parquet");
    doc.prepare_export(version, &[0, 1, 2, 3], FileType::Parquet)
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

fn filtered(doc: &Document) -> u64 {
    doc.apply_view(&ViewState {
        sort: vec![SortKey {
            column: 0,
            descending: true,
        }],
        filters: vec![Filter::Range {
            include_nulls: false,
            column: 0,
            min: Some(doc.parse_bound(0, "2").unwrap()),
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
            FileType::Csv,
            concat!(
                "\"text,\"\"name\",id,value,date\r\n",
                ",9007199254740993,2.2250738585072014e-308,1969-12-31\r\n",
                ",4,,1970-01-02\r\n",
                "\"a,\"\"雪\"\"\nline\",3,1.2345678901234567,1970-01-01\r\n",
                "\"tab\tcell\rtail\",2,-0.0,1970-01-03\r\n",
            ),
        ),
        (
            FileType::Tsv,
            concat!(
                "\"text,\"\"name\"\tid\tvalue\tdate\r\n",
                "\t9007199254740993\t2.2250738585072014e-308\t1969-12-31\r\n",
                "\t4\t\t1970-01-02\r\n",
                "\"a,\"\"雪\"\"\nline\"\t3\t1.2345678901234567\t1970-01-01\r\n",
                "\"tab\tcell\rtail\"\t2\t-0.0\t1970-01-03\r\n",
            ),
        ),
    ] {
        let path = directory.path().join(format!("result.{format:?}"));
        let export = doc.prepare_export(version, &[1, 0, 2, 3], format).unwrap();
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
        .prepare_export(version, &[1, 0, 2, 3], FileType::Parquet)
        .unwrap();
    // Query changes and cancellation in the source do not alter the snapshot.
    doc.apply_view(&ViewState::default()).unwrap();
    doc.cancel_pending();
    let path = directory.path().join("result.parquet");
    export.write(&path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap()).unwrap();
    assert!(builder
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|group| group.columns())
        .all(|column| { matches!(column.compression(), parquet::basic::Compression::ZSTD(_)) }));
    let mut reader = builder.build().unwrap();
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
fn parquet_export_honors_explicit_compression() {
    let (directory, doc) = sample();
    let path = directory.path().join("snappy.parquet");
    doc.prepare_export(0, &[0, 1], FileType::Parquet)
        .unwrap()
        .write_with_options(&path, &ExportOptions {
            parquet_compression: ParquetCompression::Snappy,
        })
        .unwrap();
    let reopened = Document::open(path).unwrap();
    let info = reopened.info();
    let gewu_core::FileMetadata::Parquet { compression, .. } = &info.metadata else {
        panic!("expected Parquet metadata");
    };
    assert_eq!(compression, &["SNAPPY"]);
    assert_eq!(
        reopened.rows(0, 0, 5, &[0, 1]).unwrap().cells,
        doc.rows(0, 0, 5, &[0, 1]).unwrap().cells
    );
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
    for format in [FileType::Csv, FileType::Tsv, FileType::Parquet] {
        let path = directory.path().join(format!("empty.{format:?}"));
        doc.prepare_export(version, &[0], format)
            .unwrap()
            .write(&path)
            .unwrap();
        if format == FileType::Parquet {
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
        doc.prepare_export(old, &[0], FileType::Csv),
        Err(Error::StaleVersion { .. })
    ));
    for columns in [vec![], vec![0, 0], vec![99]] {
        assert!(matches!(
            doc.prepare_export(version, &columns, FileType::Csv),
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
    let export = doc.prepare_export(version, &[0], FileType::Csv).unwrap();
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
    let export = doc.prepare_export(version, &[0], FileType::Csv).unwrap();
    assert!(matches!(export.write(&target_directory), Err(Error::Io(_))));
    assert!(target_directory.is_dir());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".gewu-export-")));
    // Successful export can replace a destination once, after all rows exist.
    let export = doc.prepare_export(version, &[0], FileType::Csv).unwrap();
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
        .prepare_export(doc.current_view().version, &[0], FileType::Csv)
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
            .prepare_export(doc.current_view().version, &[0], FileType::Parquet)
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
            include_nulls: false,
            column: 0,
            min: Some(doc.parse_bound(0, "2").unwrap()),
            max: Some(doc.parse_bound(0, "4").unwrap()),
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
