#![cfg(feature = "csv")]

use gewu_core::{AnalysisData, AnalysisSection, Document, Error, Filter, ViewState};

#[test]
fn analysis_keeps_its_snapshot_and_cancels_independently_of_queries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("analysis.csv");
    std::fs::write(&path, "number,text\n1,a\n2,b\n3,b\n").unwrap();
    let doc = Document::open(path).unwrap();
    let original = doc.current_view();
    let old = doc
        .prepare_analysis(original.version, 0, AnalysisSection::Statistics)
        .unwrap();
    let scatter = doc.prepare_scatter(original.version, None, 0, 8).unwrap();
    let cancelled = doc
        .prepare_analysis(original.version, 1, AnalysisSection::Values)
        .unwrap();
    cancelled.cancel();
    let current = doc
        .apply_view(&ViewState::single(1, Filter::Contains {
            text: "b".into(),
            case_sensitive: true,
            include_nulls: false,
        }))
        .unwrap();
    assert_eq!(current.row_count, 2);
    assert!(matches!(cancelled.run(), Err(Error::Cancelled)));
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
        doc.prepare_analysis(original.version, 0, AnalysisSection::Overview),
        Err(Error::StaleVersion { .. })
    ));
    let result = old.run().unwrap();
    assert_eq!((result.version, result.column), (original.version, 0));
    let AnalysisData::Statistics { statistics } = result.data else {
        panic!("statistics")
    };
    assert_eq!(statistics.mean, Some(2.0));
    assert_eq!(statistics.min_label.as_deref(), Some("1"));
    let current = doc
        .prepare_analysis(current.version, 0, AnalysisSection::Statistics)
        .unwrap()
        .run()
        .unwrap();
    let AnalysisData::Statistics { statistics } = current.data else {
        panic!("statistics")
    };
    assert_eq!(statistics.mean, Some(2.5));
    assert_eq!(statistics.median_label.as_deref(), Some("2.5"));
    assert!(matches!(
        doc.prepare_analysis(doc.current_view().version, 1, AnalysisSection::Statistics),
        Err(Error::InvalidArgument(_))
    ));
}
