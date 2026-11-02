use super::*;
use crate::{ColSpec, FilterCondition};

fn query(column: u32, filter: Filter) -> QuerySpec {
    QuerySpec {
        columns: vec![ColSpec {
            column,
            filter: Some(filter),
            sort: None,
        }],
    }
}

#[test]
fn typed_filters_compile_and_execute_across_ffi() {
    let source = std::env::temp_dir().join(format!("gewu-uniffi-typed-{}.csv", std::process::id()));
    std::fs::write(&source, "id\n9007199254740992\n9007199254740993\n").unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    assert!(doc.info().columns[0].capabilities.histogram);
    assert!(doc.info().columns[0].capabilities.scatter);
    let points = doc
        .view()
        .analyze_scatter(
            None,
            0,
            4,
            AxisScale::Linear,
            AxisScale::Linear,
            Cancellation::new(),
        )
        .unwrap();
    assert_eq!(points.eligible_rows, 2);
    assert!(!points.sampled);
    assert_eq!(points.points[1].source_row, 1);
    assert_eq!(points.points[1].y_label, "9007199254740993");
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(matches!(
        doc.view()
            .analyze_scatter(None, 0, 4, AxisScale::Linear, AxisScale::Linear, cancelled),
        Err(GewuError::Cancelled)
    ));
    let statistics = doc
        .view()
        .analyze_statistics(0, crate::Cancellation::new())
        .unwrap();
    assert_eq!(
        statistics.median_label.as_deref(),
        Some("9007199254740992.5")
    );
    assert!(!doc.info().columns[0].capabilities.contains);
    // Chart coordinates may round a large ID; typed filters below must not.
    let before = doc.view();
    assert!(matches!(
        before.query(
            query(0, Filter {
                condition: FilterCondition::Range {
                    min: Some("not a number".into()),
                    max: None
                },
                include_nulls: false,
            }),
            crate::Cancellation::new()
        ),
        Err(GewuError::InvalidArgument { .. })
    ));
    let values = doc
        .view()
        .filter_choices(0, crate::Cancellation::new())
        .unwrap()
        .values
        .unwrap();
    assert_eq!(values[1].value, "9007199254740993");
    assert_eq!(before.row_count(), 2);
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
        let view = doc
            .view()
            .query(
                query(0, Filter {
                    condition,
                    include_nulls: false,
                }),
                Cancellation::new(),
            )
            .unwrap();
        let rows = view.rows(0, 10, vec![0]).unwrap();
        assert_eq!(rows.source_rows, [1]);
        assert_eq!(rows.cells, [Some("9007199254740993".into())]);
    }
    std::fs::remove_file(source).unwrap();
}

#[test]
fn text_filter_modes_and_paged_choices_cross_ffi() {
    let source = std::env::temp_dir().join(format!(
        "gewu-uniffi-filter-spec-{}.csv",
        std::process::id()
    ));
    std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
    let doc = GewuDataset::open(source.to_str().unwrap().to_owned()).unwrap();
    let info = doc.info();
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
    assert!(info.columns[0].capabilities.contains);
    assert!(info.columns[0].capabilities.exact);
    let state = query(1, Filter {
        condition: FilterCondition::Exact {
            values: vec!["10".into()],
            exclude: false,
        },
        include_nulls: false,
    });
    let choices = doc
        .view()
        .query(state, Cancellation::new())
        .unwrap()
        .text_choices(0)
        .unwrap();
    let first = choices
        .page(String::new(), None, 1, crate::Cancellation::new())
        .unwrap();
    assert_eq!(first.values.len(), 1);
    assert_eq!(first.values[0].value, "cat");
    assert_eq!(first.values[0].count, 1);
    assert!(
        first.next.is_none(),
        "Only current-view values are candidates"
    );
    let view = doc
        .view()
        .query(
            query(0, Filter {
                condition: FilterCondition::Exact {
                    values: vec!["dog".into()],
                    exclude: false,
                },
                include_nulls: false,
            }),
            Cancellation::new(),
        )
        .unwrap();
    assert_eq!(view.rows(0, 10, vec![0]).unwrap().cells, [Some(
        "dog".into()
    )]);
    choices.cancel();
    assert!(matches!(
        choices.page(String::new(), None, 1, crate::Cancellation::new()),
        Err(GewuError::Cancelled)
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
    let custom = doc
        .view()
        .filter_distribution(0, 10, AxisScale::Linear, crate::Cancellation::new())
        .unwrap()
        .unwrap();
    assert_eq!(custom.histogram.counts.len(), 10);
    assert_eq!(custom.histogram.counts.iter().sum::<u64>(), 1_000);
    assert!(matches!(
        doc.view()
            .filter_distribution(0, 0, AxisScale::Linear, crate::Cancellation::new()),
        Err(GewuError::InvalidArgument { .. })
    ));
    let distribution = doc
        .view()
        .filter_distribution(0, 60, AxisScale::Linear, crate::Cancellation::new())
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
            .view()
            .query(
                query(0, Filter {
                    condition: FilterCondition::Range {
                        min: bounds.min.as_ref().map(|b| b.value.clone()),
                        max: bounds.max.as_ref().map(|b| b.value.clone()),
                    },
                    include_nulls: false,
                }),
                Cancellation::new(),
            )
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
    assert_eq!(info.format, "CSV");
    assert_eq!(info.row_count, 2);
    assert_eq!(info.row_groups, None);
    assert!(info.compression.is_empty());
    assert!(info.sheet.is_none());
    assert!(matches!(
        crate::xlsx_sheet_names(source.to_str().unwrap().to_owned()),
        Err(GewuError::InvalidArgument { .. })
    ));
    assert!(matches!(
        GewuDataset::open_sheet(source.to_str().unwrap().to_owned(), "Sheet1".into()),
        Err(GewuError::InvalidArgument { .. })
    ));

    for (format, extension, name) in [
        (FileType::Xlsx, "xlsx", "XLSX"),
        (FileType::Parquet, "parquet", "Parquet"),
    ] {
        let destination = source.with_extension(extension);
        doc.view()
            .export(vec![0, 1], format)
            .unwrap()
            .write(destination.to_str().unwrap().to_owned())
            .unwrap();
        let reopened = GewuDataset::open(destination.to_str().unwrap().to_owned()).unwrap();
        let info = reopened.info();
        assert_eq!(info.format, name);
        assert_eq!(info.row_count, 2);
        if extension == "xlsx" {
            assert_eq!(info.row_groups, None);
            let sheet = info.sheet.unwrap();
            let path = destination.to_str().unwrap().to_owned();
            assert_eq!(crate::xlsx_sheet_names(path.clone()).unwrap(), info.sheets);
            assert_eq!(info.sheets, vec![sheet.clone()]);
            let selected = GewuDataset::open_sheet(path.clone(), sheet.clone()).unwrap();
            assert_eq!(selected.info().sheet, Some(sheet));
            assert_eq!(selected.view().rows(0, 1, vec![0]).unwrap().cells, [Some(
                "cat".into()
            )]);
            let filtered = selected
                .view()
                .query(
                    query(0, Filter {
                        condition: FilterCondition::Exact {
                            values: vec!["cat".into()],
                            exclude: false,
                        },
                        include_nulls: false,
                    }),
                    Cancellation::new(),
                )
                .unwrap();
            assert_eq!(filtered.row_count(), 1);
            assert_eq!(reopened.view().row_count(), 2);
            assert!(matches!(
                GewuDataset::open_sheet(path, "absent".into()),
                Err(GewuError::InvalidArgument { .. })
            ));
        } else {
            assert_eq!(info.row_groups, Some(1));
            assert!(!info.compression.is_empty());
            assert!(info.sheet.is_none());
        }
        std::fs::remove_file(destination).unwrap();
    }
    std::fs::remove_file(source).unwrap();
}
