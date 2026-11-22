use std::io::Write;

use gewu_core::{ColumnId, ColumnSort};

use super::*;

#[test]
fn unchanged_rows_share_results_and_failed_queries_preserve_the_snapshot() {
    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    write!(file, "score\n3\n1\n").unwrap();
    let dataset = Arc::new(Dataset::open(file.path()).unwrap());
    let score = dataset.column_id("score").unwrap();
    let original = AppliedView::new(dataset);
    let same = original.applying(Arc::default()).unwrap();
    assert!(Arc::ptr_eq(original.view(), same.view()));
    let statistics = original.analyses().statistics(score).unwrap();
    let sorted = original
        .applying(Arc::new(TableQuery {
            sorts: vec![ColumnSort {
                column: score,
                descending: false,
            }],
            ..Default::default()
        }))
        .unwrap();
    assert!(Arc::ptr_eq(original.filtered(), sorted.filtered()));
    assert!(Arc::ptr_eq(
        &statistics,
        &sorted.analyses().statistics(score).unwrap()
    ));
    assert_eq!(sorted.view().source_rows(0, 2), [1, 0]);
    let invalid = TableQuery {
        sorts: vec![ColumnSort {
            column: ColumnId::from_raw(100),
            descending: false,
        }],
        ..Default::default()
    };
    assert!(sorted.applying(Arc::new(invalid)).is_err());
    assert_eq!(sorted.view().source_rows(0, 2), [1, 0]);
}
