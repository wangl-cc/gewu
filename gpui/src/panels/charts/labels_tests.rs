use super::*;
#[test]
fn original_numeric_precision_and_differing_offsets_survive_compaction() {
    assert_eq!(compact("9007199254740993"), "9.007…e15");
    assert_eq!(compact("-0.0000000000012345"), "-1.234…e-12");
    assert_eq!(compact("1.234567890123456e100"), "1.234…e100");
    let span = endpoints(
        "2024-01-01T12:03:00.123456789Z",
        "2024-01-01T12:03:00.123456790Z",
    );
    assert_eq!(
        (
            span.lower.as_str(),
            span.upper.as_str(),
            span.context.as_deref()
        ),
        (
            "12:03:00.123456789",
            "12:03:00.123456790",
            Some("2024-01-01 · UTC")
        )
    );
    let dst = endpoints("2024-10-27T01:30:00+01:00", "2024-10-27T01:30:00+00:00");
    assert_eq!(dst.lower, "01:30:00 +01:00");
    assert_eq!(dst.upper, "01:30:00 +00:00");
    assert_eq!(significant_digits(1.23456789e-9, 4), "1.235e-9");
}
