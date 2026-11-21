//! Behavior this crate owns: error projection, source metadata records and
//! view reuse. Core owns the data semantics behind them.

use std::sync::Arc;

use gewu_core::{
    self as core, ColumnFilter, ColumnId, ColumnSort, ExportOptions, FileType, Filter,
    ValueParseError,
};

use super::*;

/// A CSV file removed when dropped.
struct Source(std::path::PathBuf);

impl Source {
    fn new(name: &str, extension: &str, contents: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gewu-uniffi-{name}-{}.{extension}",
            std::process::id()
        ));
        std::fs::write(&path, contents).unwrap();
        Self(path)
    }

    fn path(&self) -> String {
        self.0.to_str().unwrap().to_owned()
    }

    fn open(&self, options: OpenOptions) -> Result<Arc<GewuDataset>, GewuError> {
        GewuDataset::open(self.path(), options)
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

fn exact(column: u32, value: &str) -> ColumnFilter {
    ColumnFilter {
        column: ColumnId::from_raw(column),
        filter: Filter::exact(vec![value.into()], false, false),
    }
}

#[test]
fn errors_keep_their_structured_kinds() {
    let source = Source::new("errors", "csv", "id\n9007199254740993\n");
    let rows = source.open(OpenOptions::default()).unwrap().rows();
    let invalid = ColumnFilter {
        column: ColumnId::from_raw(0),
        filter: Filter::range(Some("not a number".into()), None, false),
    };
    assert!(matches!(
        rows.apply(vec![invalid], vec![]),
        Err(GewuError::Filter {
            kind: FilterErrorKind::InvalidValue {
                value,
                expected_type,
                reason: ValueParseError::InvalidSyntax,
            },
            ..
        }) if value == "not a number" && expected_type == "Int64"
    ));
    assert!(matches!(
        source.open(OpenOptions {
            sheet: Some("Sheet1".into()),
            has_header: true,
        }),
        Err(GewuError::Internal { .. })
    ));

    let malformed = Source::new("malformed", "csv", "id,label\n1\n");
    assert!(matches!(
        malformed.open(OpenOptions::default()),
        Err(GewuError::File {
            kind: FileErrorKind::Decode {
                format: FileType::Csv
            },
            ..
        })
    ));
}

#[test]
fn filter_failures_inside_analyses_keep_their_filter_kind() {
    let error = GewuError::from(core::Error::Analysis(core::AnalysisError::Filter(
        core::FilterError::ReversedRange {
            min: "9".into(),
            max: "1".into(),
        },
    )));
    assert!(matches!(error, GewuError::Filter {
        kind: FilterErrorKind::ReversedRange { .. },
        ..
    }));
}

#[test]
fn source_metadata_crosses_as_records() {
    let source = Source::new("metadata", "csv", "label,id\ncat,10\ndog,20\n");
    let rows = source.open(OpenOptions::default()).unwrap().rows();
    let columns = vec![ColumnId::from_raw(0), ColumnId::from_raw(1)];
    let options = ExportOptions::default();
    for format in [FileType::Xlsx, FileType::Parquet] {
        let destination = Source::new("metadata-export", &format!("{format:?}"), "");
        rows.export(columns.clone(), format)
            .unwrap()
            .write(destination.path(), options)
            .unwrap();
        let info = destination.open(OpenOptions::default()).unwrap().info();
        assert_eq!((info.format, info.row_count), (format, 2));
        match info.metadata {
            FileMetadata::Xlsx { sheet, sheets } => {
                assert_eq!(xlsx_sheet_names(destination.path()).unwrap(), sheets);
                assert_eq!(sheets, [sheet]);
                assert!(matches!(
                    destination.open(OpenOptions {
                        sheet: Some("absent".into()),
                        has_header: true,
                    }),
                    Err(GewuError::File {
                        kind: FileErrorKind::SheetNotFound { .. },
                        ..
                    })
                ));
            }
            FileMetadata::Parquet {
                row_groups,
                compression,
                ..
            } => assert!(row_groups == 1 && !compression.is_empty()),
            FileMetadata::Delimited => panic!("{format:?} reopened as delimited"),
        }
    }
}

#[test]
fn applying_a_query_reuses_what_did_not_change() {
    let source = Source::new("apply", "csv", "label,id\ncat,10\ndog,20\ncat,30\n");
    let all = source.open(OpenOptions::default()).unwrap().rows();
    let by_id = vec![ColumnSort {
        column: ColumnId::from_raw(1),
        descending: true,
    }];

    let same = Arc::clone(&all).apply(vec![], vec![]).unwrap();
    assert!(Arc::ptr_eq(&same, &all), "nothing changed");
    assert!(all.is_same_view(Arc::clone(&same)));

    let sorted = Arc::clone(&all).apply(vec![], by_id.clone()).unwrap();
    assert!(
        Arc::ptr_eq(&sorted.filtered(), &all.filtered()),
        "a new order keeps the filtered rows and their analyses"
    );
    assert!(!all.is_same_view(Arc::clone(&sorted)));
    assert!(sorted.filtered().is_same_view(all.filtered()));
    assert_eq!(sorted.inner.source_rows(0, 3), [2, 1, 0]);

    let cats = Arc::clone(&sorted)
        .apply(vec![exact(0, "cat")], by_id.clone())
        .unwrap();
    assert!(!Arc::ptr_eq(&cats.filtered(), &all.filtered()));
    assert!(!cats.filtered().is_same_view(all.filtered()));
    assert_eq!(cats.inner.source_rows(0, 3), [2, 0]);
    assert_eq!(cats.filtered().row_count(), 2);

    let dogs = Arc::clone(&cats)
        .apply(vec![exact(0, "dog")], vec![])
        .unwrap();
    assert_eq!(
        dogs.inner.source_rows(0, 3),
        [1],
        "filters apply to the whole dataset, not the previous rows"
    );
}
