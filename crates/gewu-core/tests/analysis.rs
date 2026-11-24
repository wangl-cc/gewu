#![cfg(feature = "csv")]
use std::sync::Arc;

use gewu_core::{AxisScale, ColumnFilter, Dataset, Error, Filter, FilteredView};

#[test]
fn analysis_and_scatter_remain_bound_to_independent_views() {
    let file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(file.path(), "number,text\n1,a\n2,b\n3,b\n").unwrap();
    let dataset = Arc::new(Dataset::open(file.path()).unwrap());
    let c0 = dataset.column_id("number").unwrap();
    let c1 = dataset.column_id("text").unwrap();
    let original = dataset.unfiltered();
    let current = dataset
        .filter(&[ColumnFilter {
            column: c1,
            filter: Filter::contains("b".into(), true, false),
        }])
        .unwrap();
    drop(dataset);
    assert_eq!(original.statistics(c0).unwrap().mean, Some(2.0));
    assert_eq!(current.statistics(c0).unwrap().mean, Some(2.5));
    assert_eq!(
        current.statistics(c0).unwrap().median_label.as_deref(),
        Some("2.5")
    );
    let scatter = |view: &FilteredView| {
        view.scatter(None, c0, 8, AxisScale::Linear, AxisScale::Linear)
            .unwrap()
    };
    assert_eq!(scatter(&original).eligible_rows, 3);
    assert_eq!(
        scatter(&current)
            .points
            .iter()
            .map(|p| p.source_row.0)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(original.count_nulls(c0).unwrap(), 0);
    assert!(matches!(current.statistics(c1), Err(Error::Analysis(_))));
}
