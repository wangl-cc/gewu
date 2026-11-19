use super::*;
use crate::{ColumnFilter, ColumnId, ColumnSort, FilterCondition};

fn filter_single(column: ColumnId, filter: Filter) -> Vec<ColumnFilter> {
    vec![ColumnFilter { column, filter }]
}

#[test]
fn typed_filters_compile_and_execute_across_ffi() {
    let source = std::env::temp_dir().join(format!("gewu-uniffi-typed-{}.csv", std::process::id()));
    std::fs::write(&source, "id\n9007199254740992\n9007199254740993\n").unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    let id_col = doc.column_id("id".into()).unwrap();
    assert!(doc.column_id("missing".into()).is_none());
    let info = doc.info();
    assert_eq!(info.columns[0].id, id_col);
    assert!(info.columns[0].analysis.distribution);
    assert!(info.columns[0].capabilities.scatter);
    let unfiltered = doc.unfiltered();
    let points = unfiltered
        .scatter(None, id_col, 4, AxisScale::Linear, AxisScale::Linear)
        .unwrap();
    assert_eq!(points.eligible_rows, 2);
    assert_eq!(points.eligible_rows, points.points.len() as u64);
    assert_eq!(points.points[1].source_row, 1);
    assert_eq!(points.points[1].y_label, "9007199254740993");
    let statistics = unfiltered.statistics(id_col).unwrap();
    assert_eq!(
        statistics.median_label.as_deref(),
        Some("9007199254740992.5")
    );
    assert!(!info.columns[0].capabilities.text_match);
    // Chart coordinates may round a large ID; typed filters below must not.
    assert!(matches!(
        doc.filter(filter_single(id_col, Filter {
            condition: FilterCondition::Range {
                min: Some("not a number".into()),
                max: None
            },
            include_nulls: false,
        })),
        Err(GewuError::Filter {
            kind: crate::FilterErrorKind::InvalidValue {
                value,
                expected_type,
                reason: crate::ValueParseError::InvalidSyntax,
            },
            ..
        }) if value == "not a number" && expected_type == "Int64"
    ));
    let values = unfiltered.filter_choices(id_col).unwrap().values.unwrap();
    assert_eq!(values[1].value, "9007199254740993");
    assert_eq!(unfiltered.row_count(), 2);
    for condition in [
        FilterCondition::Exact {
            values: vec![values[1].value.clone()],
            exclude: false,
        },
        FilterCondition::Range {
            min: Some(values[1].value.clone()),
            max: None,
        },
    ] {
        let filtered = doc
            .filter(filter_single(id_col, Filter {
                condition,
                include_nulls: false,
            }))
            .unwrap();
        let ordered = filtered.unsorted();
        let rows = ordered.format_column(id_col, 0, 10).unwrap();
        assert_eq!(ordered.source_rows(0, 10), [1]);
        assert_eq!(rows.utf8, b"9007199254740993");
    }
    std::fs::remove_file(source).unwrap();
}

#[test]
fn text_filter_modes_and_choices_cross_ffi() {
    let source = std::env::temp_dir().join(format!(
        "gewu-uniffi-filter-spec-{}.csv",
        std::process::id()
    ));
    std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    let info = doc.info();
    let label_col = info.columns[0].id;
    let id_col = info.columns[1].id;
    assert_eq!(doc.column_id("label".into()), Some(label_col));
    assert_eq!(doc.column_id("id".into()), Some(id_col));
    assert!(matches!(
        info.columns[0].value_category,
        crate::ValueCategory::Text
    ));
    assert!(matches!(
        info.columns[1].value_category,
        crate::ValueCategory::Number
    ));
    assert_eq!(info.columns[0].display_type, "string");
    assert_eq!(info.columns[1].display_type, "int64");
    assert!(info.columns[0].capabilities.text_match);
    assert!(info.columns[0].capabilities.exact);
    assert!(matches!(
        doc.unfiltered().statistics(label_col),
        Err(GewuError::Internal { .. })
    ));
    let equals = |case_sensitive| {
        doc.filter(filter_single(label_col, Filter {
            condition: FilterCondition::Text {
                text: "CAT".into(),
                mode: crate::TextMatch::Equals,
                case_sensitive,
            },
            include_nulls: false,
        }))
        .unwrap()
        .row_count()
    };
    assert_eq!(equals(false), 1);
    assert_eq!(equals(true), 0);
    let choices = doc
        .filter(filter_single(id_col, Filter {
            condition: FilterCondition::Exact {
                values: vec!["10".into()],
                exclude: false,
            },
            include_nulls: false,
        }))
        .unwrap()
        .filter_choices(label_col)
        .unwrap()
        .values
        .unwrap();
    let counts: Vec<_> = choices
        .iter()
        .map(|c| (c.value.as_str(), c.count))
        .collect();
    assert_eq!(
        counts,
        [("cat", 1)],
        "Only current-view values are candidates"
    );
    let filtered = doc
        .filter(filter_single(label_col, Filter {
            condition: FilterCondition::Exact {
                values: vec!["dog".into()],
                exclude: false,
            },
            include_nulls: false,
        }))
        .unwrap();
    assert_eq!(
        filtered
            .unsorted()
            .format_column(label_col, 0, 10)
            .unwrap()
            .utf8,
        b"dog"
    );
    // The label editor's facet drops only the label filter.
    let facet = filtered.facet(label_col).unwrap();
    let labels: Vec<_> = facet
        .filter_choices(label_col)
        .unwrap()
        .values
        .unwrap()
        .into_iter()
        .map(|c| c.value)
        .collect();
    assert_eq!(labels, ["cat", "dog"]);
    assert!(std::sync::Arc::ptr_eq(
        &filtered.facet(id_col).unwrap().inner,
        &filtered.inner
    ));
    let sorted_desc = doc
        .unfiltered()
        .sort(vec![ColumnSort {
            column: id_col,
            descending: true,
        }])
        .unwrap();
    let batch = sorted_desc
        .read_segments(
            vec![RowSpan { start: 0, count: 2 }, RowSpan {
                start: 1,
                count: 64,
            }],
            vec![
                ColumnSpan {
                    column: label_col,
                    start: 1,
                    count: 64,
                },
                ColumnSpan {
                    column: id_col,
                    start: 0,
                    count: 64,
                },
            ],
        )
        .unwrap();
    assert_eq!(batch.source_rows, [vec![1, 0], vec![0]]);
    let [label, id] = batch.segments.try_into().ok().unwrap();
    assert_eq!((label.row_count, label.utf8), (1, b"cat".to_vec()));
    assert_eq!((id.row_count, id.utf8), (2, b"2010".to_vec()));
    let offsets: Vec<u8> = [0u32, 2, 4]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert_eq!(id.offsets, offsets);
    assert!(sorted_desc
        .read_segments(vec![], vec![ColumnSpan {
            column: ColumnId { raw: 99 },
            start: 0,
            count: 1,
        }])
        .is_err());
    let selected = |format| {
        sorted_desc
            .export_rows(
                vec![RowSpan { start: 1, count: 5 }],
                vec![label_col, id_col],
                format,
            )
            .unwrap()
    };
    assert_eq!(
        selected(crate::FileType::Tsv)
            .write_text(true, 1024)
            .unwrap(),
        b"label\tid\ncat\t10\n"
    );
    assert!(matches!(
        selected(crate::FileType::Tsv).write_text(false, 2),
        Err(GewuError::Limit {
            kind: crate::LimitErrorKind::TextBytes { limit: 2 },
            ..
        })
    ));
    assert!(matches!(
        selected(crate::FileType::Parquet).write_text(true, 1024),
        Err(GewuError::Export {
            kind: crate::ExportErrorKind::TextFormat { .. },
            ..
        })
    ));
    std::fs::remove_file(source).unwrap();
}

#[test]
fn histogram_bounds_cross_ffi_without_losing_large_ids() {
    use std::fmt::Write;

    let source =
        std::env::temp_dir().join(format!("gewu-uniffi-range-spec-{}.csv", std::process::id()));
    let base = 9_007_199_254_740_993_i64;
    let mut csv = String::from("id\n");
    for offset in 0..1_000 {
        writeln!(csv, "{}", base + offset).unwrap();
    }
    std::fs::write(&source, csv).unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    let id_col = doc.column_id("id".into()).unwrap();
    let unfiltered = doc.unfiltered();
    let custom = unfiltered
        .filter_distribution(id_col, 10, AxisScale::Linear)
        .unwrap()
        .unwrap();
    assert_eq!(custom.histogram.counts.len(), 10);
    assert_eq!(custom.histogram.counts.iter().sum::<u64>(), 1_000);
    assert!(matches!(
        unfiltered.filter_distribution(id_col, 0, AxisScale::Linear),
        Err(GewuError::Internal { .. })
    ));
    let distribution = unfiltered
        .filter_distribution(id_col, 60, AxisScale::Linear)
        .unwrap()
        .unwrap();
    assert_eq!(distribution.histogram.min_label, Some(base.to_string()));
    assert_eq!(
        distribution.histogram.max_label,
        Some((base + 999).to_string())
    );
    assert_eq!(
        distribution.ranges.len(),
        distribution.histogram.counts.len()
    );
    assert!(distribution.ranges.first().unwrap().min.is_none());
    assert!(distribution.ranges.last().unwrap().max.is_none());
    for bin in [
        0,
        distribution.ranges.len() / 2,
        distribution.ranges.len() - 1,
    ] {
        let bounds = &distribution.ranges[bin];
        let view = doc
            .filter(filter_single(id_col, Filter {
                condition: FilterCondition::Range {
                    min: bounds.min.as_ref().map(|b| b.value.clone()),
                    max: bounds.max.as_ref().map(|b| b.value.clone()),
                },
                include_nulls: false,
            }))
            .unwrap();
        assert_eq!(view.row_count(), distribution.histogram.counts[bin]);
    }
    std::fs::remove_file(source).unwrap();
}

#[test]
fn file_metadata_and_xlsx_export_cross_the_ffi_boundary() {
    let source =
        std::env::temp_dir().join(format!("gewu-uniffi-formats-{}.csv", std::process::id()));
    std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    let info = doc.info();
    let label_col = info.columns[0].id;
    let id_col = info.columns[1].id;
    assert_eq!(info.format, FileType::Csv);
    assert_eq!(info.row_count, 2);
    assert!(matches!(info.metadata, FileMetadata::Delimited));
    assert!(matches!(
        crate::xlsx_sheet_names(source.to_str().unwrap().to_owned()),
        Err(GewuError::Internal { .. })
    ));
    assert!(matches!(
        GewuDataset::open_sheet(source.to_str().unwrap().to_owned(), "Sheet1".into()),
        Err(GewuError::Internal { .. })
    ));

    for (format, extension) in [(FileType::Xlsx, "xlsx"), (FileType::Parquet, "parquet")] {
        let destination = source.with_extension(extension);
        doc.unfiltered()
            .unsorted()
            .export(vec![label_col, id_col], format)
            .unwrap()
            .write(destination.to_str().unwrap().to_owned())
            .unwrap();
        let reopened = GewuDataset::open(destination.to_str().unwrap().to_owned()).unwrap();
        let info = reopened.info();
        assert_eq!(info.format, format);
        assert_eq!(info.row_count, 2);
        if extension == "xlsx" {
            let FileMetadata::Xlsx { sheet, sheets } = info.metadata else {
                panic!("expected XLSX metadata")
            };
            let path = destination.to_str().unwrap().to_owned();
            assert_eq!(crate::xlsx_sheet_names(path.clone()).unwrap(), sheets);
            assert_eq!(sheets, vec![sheet.clone()]);
            let selected = GewuDataset::open_sheet(path.clone(), sheet.clone()).unwrap();
            let selected_label = selected.column_id("label".into()).unwrap();
            assert!(matches!(
                selected.info().metadata,
                FileMetadata::Xlsx { sheet: selected, .. } if selected == sheet
            ));
            assert_eq!(
                selected
                    .unfiltered()
                    .unsorted()
                    .format_column(selected_label, 0, 1)
                    .unwrap()
                    .utf8,
                b"cat"
            );
            let filtered = selected
                .filter(filter_single(selected_label, Filter {
                    condition: FilterCondition::Exact {
                        values: vec!["cat".into()],
                        exclude: false,
                    },
                    include_nulls: false,
                }))
                .unwrap();
            assert_eq!(filtered.row_count(), 1);
            assert_eq!(reopened.unfiltered().row_count(), 2);
            assert!(matches!(
                GewuDataset::open_sheet(path, "absent".into()),
                Err(GewuError::File {
                    kind: crate::FileErrorKind::SheetNotFound { .. },
                    ..
                })
            ));
        } else {
            let FileMetadata::Parquet {
                row_groups,
                compression,
                ..
            } = info.metadata
            else {
                panic!("expected Parquet metadata")
            };
            assert_eq!(row_groups, 1);
            assert!(!compression.is_empty());
        }
        std::fs::remove_file(destination).unwrap();
    }
    std::fs::remove_file(source).unwrap();
}

#[test]
fn malformed_csv_is_a_format_error_across_ffi() {
    let source =
        std::env::temp_dir().join(format!("gewu-uniffi-invalid-{}.csv", std::process::id()));
    std::fs::write(&source, "id,label\n1\n").unwrap();
    assert!(matches!(
        GewuDataset::open(source.to_str().unwrap().to_owned()),
        Err(GewuError::File {
            kind: crate::FileErrorKind::Decode {
                format: FileType::Csv
            },
            ..
        })
    ));
    std::fs::remove_file(source).unwrap();
}
