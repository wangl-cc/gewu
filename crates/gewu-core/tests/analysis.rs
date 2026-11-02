#![cfg(feature = "csv")]

use gewu_core::{Cancellation, Error, Filter, QuerySpec, Session};

#[test]
fn analysis_checks_version_at_execution_and_cancels_independently_of_queries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("analysis.csv");
    std::fs::write(&path, "number,text\n1,a\n2,b\n3,b\n").unwrap();
    let doc = Session::open(path).unwrap();
    let original = doc.current_view();
    let old = doc
        .analyze_statistics(original.version, 0, &Cancellation::default())
        .unwrap();
    assert_eq!(old.mean, Some(2.0));
    let scatter = doc.prepare_scatter(original.version, None, 0, 8).unwrap();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let current = doc
        .prepare_view(&QuerySpec::single(
            1,
            Filter::contains("b".into(), true, false),
        ))
        .and_then(|operation| operation.run())
        .unwrap();
    assert_eq!(current.row_count, 2);
    assert!(matches!(
        doc.analyze_values(original.version, 1, &cancelled),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        doc.prepare_scatter(original.version, None, 0, 8),
        Err(Error::StaleVersion { .. })
    ));
    assert_eq!(scatter.run().unwrap().total_rows, 3);
    let plotted = doc
        .prepare_scatter(current.version, None, 0, 8)
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(
        plotted
            .points
            .iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(matches!(
        doc.analyze_overview(original.version, 0, &Cancellation::default()),
        Err(Error::StaleVersion { .. })
    ));
    let statistics = doc
        .analyze_statistics(current.version, 0, &Cancellation::default())
        .unwrap();
    assert_eq!(statistics.mean, Some(2.5));
    assert_eq!(statistics.median_label.as_deref(), Some("2.5"));
    assert!(matches!(
        doc.analyze_statistics(doc.current_view().version, 1, &Cancellation::default()),
        Err(Error::InvalidArgument(_))
    ));
}
