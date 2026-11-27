use gewu_core::{AnalysisError, Error, IoError, IoOperation};

#[test]
fn opening_a_file_and_an_unavailable_analysis_get_distinct_messages() {
    let denied = Error::from(IoError {
        operation: IoOperation::Open,
        path: "measurements.csv".into(),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
    });
    assert_eq!(
        super::message(&denied),
        "Could not open measurements.csv. permission denied"
    );
    assert_eq!(
        super::message(&Error::from(AnalysisError::NoFiniteValues)),
        "There are no finite values to analyze."
    );
}

#[test]
fn a_search_beyond_the_matchers_compilation_limit_has_a_readable_error() {
    let source = regex::RegexBuilder::new("[a-z]{100}")
        .size_limit(1)
        .build()
        .unwrap_err();
    let error = gewu_core::FindError::QueryCompilationFailed { source };
    assert_eq!(
        super::message(&error.into()),
        "The search text is too complex to match."
    );
}

#[test]
fn invalid_filter_during_analysis_retains_the_filter_message() {
    let error = gewu_core::AnalysisError::Filter(gewu_core::FilterError::ReversedRange {
        min: "11".into(),
        max: "4".into(),
    });
    assert_eq!(
        super::message(&error.into()),
        "The minimum 11 is greater than the maximum 4."
    );
}
