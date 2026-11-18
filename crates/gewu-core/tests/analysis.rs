#![cfg(feature = "csv")]
use std::sync::Arc;

use gewu_core::{AxisScale, Dataset, Error, Filter, QuerySpec};

#[test]
fn analysis_and_scatter_remain_bound_to_independent_views() {
    let file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(file.path(), "number,text\n1,a\n2,b\n3,b\n").unwrap();
    let dataset = Arc::new(Dataset::open(file.path()).unwrap());
    let original = dataset.view();
    let current = dataset
        .query(&QuerySpec::all(2).with_filter(1, Filter::contains("b".into(), true, false)))
        .unwrap();
    drop(dataset);
    assert_eq!(original.analyze_statistics(0).unwrap().mean, Some(2.0));
    assert_eq!(current.analyze_statistics(0).unwrap().mean, Some(2.5));
    assert_eq!(
        current
            .analyze_statistics(0)
            .unwrap()
            .median_label
            .as_deref(),
        Some("2.5")
    );
    let scatter = |view: &gewu_core::View| {
        view.analyze_scatter(None, 0, 8, AxisScale::Linear, AxisScale::Linear)
            .unwrap()
    };
    assert_eq!(scatter(&original).eligible_rows, 3);
    assert_eq!(
        scatter(&current)
            .points
            .iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(original.count_nulls(0).unwrap(), 0);
    assert!(matches!(
        current.analyze_statistics(1),
        Err(Error::Analysis(_))
    ));
}
