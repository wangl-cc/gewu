use std::fs;

use gewu_core::{Document, Error, FileType, OpenOptions};

#[test]
fn feature_availability_matches_open_errors() {
    let directory = tempfile::tempdir().unwrap();
    for (format, extension, enabled) in [
        (FileType::Parquet, "parquet", cfg!(feature = "parquet")),
        (FileType::Csv, "csv", cfg!(feature = "csv")),
        (FileType::Tsv, "tsv", cfg!(feature = "csv")),
        (FileType::Xlsx, "xlsx", cfg!(feature = "xlsx")),
    ] {
        assert_eq!(format.is_enabled(), enabled);
        if !enabled {
            let path = directory.path().join(format!("disabled.{extension}"));
            fs::write(&path, []).unwrap();
            assert!(matches!(Document::open(&path), Err(Error::Unsupported(_))));
            assert!(matches!(
                Document::open_with_options(&path, &OpenOptions {
                    format: Some(format),
                    ..Default::default()
                }),
                Err(Error::Unsupported(_))
            ));
        }
    }
}

#[cfg(any(feature = "csv", feature = "xlsx"))]
fn assert_disabled_exports(doc: &Document) {
    for format in [
        FileType::Parquet,
        FileType::Csv,
        FileType::Tsv,
        FileType::Xlsx,
    ] {
        if !format.is_enabled() {
            assert!(matches!(
                doc.prepare_export(doc.current_view().version, &[0], format, Default::default()),
                Err(Error::Unsupported(_))
            ));
        }
    }
}

#[cfg(feature = "csv")]
mod delimited {
    use arrow::datatypes::DataType;
    use gewu_core::{ExportOptions, FileMetadata, Filter, SortKey, ViewState};

    use super::*;

    #[test]
    fn parquet_like_headers_do_not_override_text_extensions() {
        let directory = tempfile::tempdir().unwrap();
        for (extension, delimiter) in [("CSV", ','), ("tsv", '\t')] {
            let path = directory.path().join(format!("source.{extension}"));
            fs::write(&path, format!("PAR1_id{delimiter}label\n1{delimiter}ok\n")).unwrap();
            let doc = Document::open(path).unwrap();
            assert_eq!(doc.info().row_count, 1);
            assert_eq!(doc.column_index("PAR1_id"), Some(0));
        }
    }

    #[test]
    fn mixed_fractional_values_do_not_round_large_integer_literals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mixed.csv");
        fs::write(&path, "id,ordinary\n9007199254740993,42\n1.5,2.5\n").unwrap();
        let doc = Document::open(path).unwrap();
        assert_eq!(doc.schema().field(0).data_type(), &DataType::Utf8);
        assert_eq!(doc.schema().field(1).data_type(), &DataType::Float64);
        assert_eq!(doc.rows(0, 0, 2, &[0]).unwrap().cells, [
            Some("9007199254740993".into()),
            Some("1.5".into())
        ]);
    }

    #[test]
    fn reads_quoted_multiline_unicode_and_infers_types_without_headers() {
        let directory = tempfile::tempdir().unwrap();
        for (extension, delimiter, format) in
            [("csv", ',', FileType::Csv), ("tsv", '\t', FileType::Tsv)]
        {
            let path = directory.path().join(format!("source.{extension}"));
            let text = format!("7{delimiter}true{delimiter}1.25{delimiter}\"雪{delimiter}\"\"quoted\"\"\nline\"\n8{delimiter}false{delimiter}{delimiter}plain\n");
            fs::write(&path, text).unwrap();
            let doc = Document::open_with_options(&path, &OpenOptions {
                has_header: false,
                ..Default::default()
            })
            .unwrap();
            assert_eq!(doc.info().format, format);
            assert_eq!(doc.info().metadata, FileMetadata::Delimited);
            assert_eq!(doc.info().row_count, 2);
            assert_eq!(doc.schema().field(0).data_type(), &DataType::Int64);
            assert_eq!(doc.schema().field(1).data_type(), &DataType::Boolean);
            assert_eq!(doc.schema().field(2).data_type(), &DataType::Float64);
            let rows = doc
                .rows(doc.current_view().version, 0, 2, &[0, 1, 2, 3])
                .unwrap();
            assert_eq!(rows.cell(0, 0), Some("7"));
            assert_eq!(rows.cell(0, 1), Some("true"));
            assert_eq!(rows.cell(0, 2), Some("1.25"));
            assert_eq!(
                rows.cell(0, 3),
                Some(format!("雪{delimiter}\"quoted\"\nline").as_str())
            );
            assert_eq!(rows.cell(1, 2), None);
            assert_disabled_exports(&doc);
        }
    }

    #[test]
    fn inference_includes_values_beyond_the_first_record_batch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mixed.csv");
        let mut contents = String::from("value\n");
        for _ in 0..70_000 {
            contents.push_str("1\n");
        }
        contents.push_str("late text\n");
        fs::write(&path, contents).unwrap();
        let doc = Document::open(path).unwrap();
        assert_eq!(doc.schema().field(0).data_type(), &DataType::Utf8);
        assert_eq!(doc.info().row_count, 70_001);
        let rows = doc
            .rows(doc.current_view().version, 69_999, 2, &[0])
            .unwrap();
        assert_eq!(rows.cell(0, 0), Some("1"));
        assert_eq!(rows.cell(1, 0), Some("late text"));
    }

    #[test]
    fn explicit_format_supports_custom_extensions_and_invalid_rows_fail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("table.custom");
        fs::write(&path, "id\tlabel\n1\t雪\n").unwrap();
        assert!(matches!(Document::open(&path), Err(Error::Unsupported(_))));
        let options = OpenOptions {
            format: Some(FileType::Tsv),
            ..Default::default()
        };
        let doc = Document::open_with_options(&path, &options).unwrap();
        assert_eq!(doc.info().format, FileType::Tsv);
        assert_eq!(doc.column_index("label"), Some(1));
        assert_eq!(doc.info().row_count, 1);
        assert!(matches!(
            Document::open_with_options(&path, &OpenOptions {
                sheet: Some("Sheet1".into()),
                ..options.clone()
            }),
            Err(Error::InvalidArgument(_))
        ));
        for contents in ["a\tb\n1\t2\t3\n", "a\tb\n1\n"] {
            fs::write(&path, contents).unwrap();
            assert!(Document::open_with_options(&path, &options).is_err());
        }
    }

    #[test]
    fn selected_sorted_rows_and_empty_headers_reload_in_both_formats() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.csv");
        fs::write(&source, "id,label,enabled\n3,\"雪,\"\"quoted\"\"\nline\",true\n1,omit,false\n2,\"tab\tcell\",false\n").unwrap();
        let doc = Document::open(source).unwrap();
        let version = doc
            .apply_view(&ViewState {
                filters: vec![Filter::Range {
                    column: 0,
                    min: Some(doc.parse_bound(0, "2").unwrap()),
                    max: None,
                }],
                sort: vec![SortKey {
                    column: 0,
                    descending: true,
                }],
                ..Default::default()
            })
            .unwrap()
            .version;
        for (format, extension) in [(FileType::Csv, "csv"), (FileType::Tsv, "tsv")] {
            let path = directory.path().join(format!("selected.{extension}"));
            let export = doc
                .prepare_export(version, &[1, 0], format, ExportOptions::default())
                .unwrap();
            export.write(&path).unwrap();
            assert_eq!(export.progress().rows_written, 2);
            let reloaded = Document::open(path).unwrap();
            assert_eq!(reloaded.info().row_count, 2);
            assert_eq!(reloaded.schema().field(0).name(), "label");
            assert_eq!(reloaded.schema().field(1).name(), "id");
            assert_eq!(
                reloaded
                    .rows(reloaded.current_view().version, 0, 2, &[0, 1])
                    .unwrap()
                    .cells,
                doc.rows(version, 0, 2, &[1, 0]).unwrap().cells
            );
        }
        let empty = doc
            .apply_view(&ViewState {
                search: Some("absent-value".into()),
                ..Default::default()
            })
            .unwrap();
        for (format, extension) in [(FileType::Csv, "csv"), (FileType::Tsv, "tsv")] {
            let path = directory.path().join(format!("empty.{extension}"));
            doc.prepare_export(empty.version, &[1, 0], format, ExportOptions::default())
                .unwrap()
                .write(&path)
                .unwrap();
            let reloaded = Document::open(path).unwrap();
            assert_eq!(reloaded.info().row_count, 0);
            assert_eq!(reloaded.schema().field(0).name(), "label");
            assert_eq!(reloaded.schema().field(1).name(), "id");
        }
    }
}

#[cfg(feature = "xlsx")]
mod xlsx {
    use std::path::Path;

    use arrow::datatypes::DataType;
    use gewu_core::{ExportOptions, FileMetadata, SortKey, ViewState};
    use rust_xlsxwriter::Workbook;

    use super::*;

    fn workbook(path: &Path) {
        let mut workbook = Workbook::new();
        let first = workbook.add_worksheet();
        first.set_name("数据").unwrap();
        first.write_row(0, 0, ["id", "label", "enabled"]).unwrap();
        first.write_number(1, 0, 2).unwrap();
        first.write_string(1, 1, "雪\nline").unwrap();
        first.write_boolean(1, 2, true).unwrap();
        first.write_number(2, 0, 1).unwrap();
        first.write_string(2, 1, "=literal").unwrap();
        first.write_boolean(2, 2, false).unwrap();
        let second = workbook.add_worksheet();
        second.set_name("Raw").unwrap();
        second.write_row(0, 0, [10, 20]).unwrap();
        second.write_row(1, 0, [30, 40]).unwrap();
        workbook.save(path).unwrap();
    }

    #[test]
    fn first_sheet_metadata_named_selection_and_header_policy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.xlsx");
        workbook(&path);
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.info().format, FileType::Xlsx);
        assert_eq!(doc.info().row_count, 2);
        assert_eq!(doc.info().metadata, FileMetadata::Xlsx {
            sheet: "数据".into(),
            sheets: vec!["数据".into(), "Raw".into()]
        });
        assert_eq!(doc.column_index("label"), Some(1));
        assert_eq!(doc.schema().field(2).data_type(), &DataType::Boolean);
        assert_eq!(
            doc.rows(doc.current_view().version, 0, 1, &[1])
                .unwrap()
                .cell(0, 0),
            Some("雪\nline")
        );
        assert_disabled_exports(&doc);
        let options = OpenOptions {
            sheet: Some("Raw".into()),
            has_header: false,
            ..Default::default()
        };
        let raw = Document::open_with_options(&path, &options).unwrap();
        assert_eq!(raw.info().row_count, 2);
        assert_eq!(raw.schema().field(0).name(), "column_1");
        assert_eq!(
            raw.rows(raw.current_view().version, 0, 2, &[0, 1])
                .unwrap()
                .cells,
            ["10.0", "20.0", "30.0", "40.0"].map(|s| Some(s.to_owned()))
        );
        assert_eq!(raw.info().metadata, FileMetadata::Xlsx {
            sheet: "Raw".into(),
            sheets: vec!["数据".into(), "Raw".into()]
        });
        let header = Document::open_with_options(&path, &OpenOptions {
            has_header: true,
            ..options
        })
        .unwrap();
        assert_eq!(header.info().row_count, 1);
        assert_eq!(header.schema().field(0).name(), "10");
        assert!(matches!(
            Document::open_with_options(path, &OpenOptions {
                sheet: Some("missing".into()),
                ..Default::default()
            }),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn export_reloads_selected_rows_and_empty_headers() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.xlsx");
        workbook(&source);
        let doc = Document::open(source).unwrap();
        let version = doc
            .apply_view(&ViewState {
                sort: vec![SortKey {
                    column: 0,
                    descending: false,
                }],
                ..Default::default()
            })
            .unwrap()
            .version;
        let destination = directory.path().join("export.xlsx");
        doc.prepare_export(version, &[1, 2], FileType::Xlsx, ExportOptions::default())
            .unwrap()
            .write(&destination)
            .unwrap();
        let reloaded = Document::open(&destination).unwrap();
        assert_eq!(
            reloaded
                .rows(reloaded.current_view().version, 0, 2, &[0, 1])
                .unwrap()
                .cells,
            doc.rows(version, 0, 2, &[1, 2]).unwrap().cells
        );
        let empty = doc
            .apply_view(&ViewState {
                search: Some("absent-value".into()),
                ..Default::default()
            })
            .unwrap();
        doc.prepare_export(
            empty.version,
            &[1, 2],
            FileType::Xlsx,
            ExportOptions::default(),
        )
        .unwrap()
        .write(&destination)
        .unwrap();
        let reloaded = Document::open(destination).unwrap();
        assert_eq!(reloaded.info().row_count, 0);
        assert_eq!(reloaded.schema().field(0).name(), "label");
        assert_eq!(reloaded.schema().field(1).name(), "enabled");
    }

    #[test]
    fn cancelled_export_and_failed_commit_preserve_destination() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.xlsx");
        workbook(&source);
        let doc = Document::open(source).unwrap();
        let destination = directory.path().join("existing.xlsx");
        fs::write(&destination, "keep me").unwrap();
        let export = doc
            .prepare_export(
                doc.current_view().version,
                &[0],
                FileType::Xlsx,
                ExportOptions::default(),
            )
            .unwrap();
        export.cancel();
        assert!(matches!(export.write(&destination), Err(Error::Cancelled)));
        assert_eq!(fs::read_to_string(destination).unwrap(), "keep me");
        let destination = directory.path().join("directory.xlsx");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), "existing data").unwrap();
        let export = doc
            .prepare_export(
                doc.current_view().version,
                &[0],
                FileType::Xlsx,
                ExportOptions::default(),
            )
            .unwrap();
        assert!(matches!(export.write(&destination), Err(Error::Io(_))));
        assert_eq!(
            fs::read_to_string(destination.join("keep")).unwrap(),
            "existing data"
        );
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".gewu-export-")));
    }
}
