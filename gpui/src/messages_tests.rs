use super::{counted, format_count, format_grouped_number, format_numeric_statistic};

#[test]
fn large_display_counts_keep_every_digit() {
    assert_eq!(format_count(0), "0");
    assert_eq!(format_count(2_000_000), "2,000,000");
    assert_eq!(format_count(u64::MAX), "18,446,744,073,709,551,615");
    assert_eq!(counted(1, "row"), "1 row");
    assert_eq!(counted(50_000, "row"), "50,000 rows");
}

#[test]
fn groups_integer_statistics_including_half_step_medians() {
    assert_eq!(format_grouped_number("999999.5"), "999,999.5");
    assert_eq!(format_grouped_number("-12345.5"), "-12,345.5");
    assert_eq!(format_grouped_number("1999999"), "1,999,999");
    assert_eq!(format_grouped_number("999.5"), "999.5");
    assert_eq!(format_grouped_number("2024-01-01"), "2024-01-01");
}

#[test]
fn formats_numeric_statistics_with_thousands_separators_before_scientific_notation() {
    assert_eq!(format_numeric_statistic(999_999.5, 5), "1,000,000");
    assert_eq!(format_numeric_statistic(577_350.413, 5), "577,350");
    assert_eq!(format_numeric_statistic(12_345.6, 6), "12,345.6");
    assert_eq!(format_numeric_statistic(-12_345.5, 6), "-12,345.5");
    assert_eq!(format_numeric_statistic(1.2345e-9, 4), "1.235e-9");
    assert_eq!(format_numeric_statistic(1.2345e12, 4), "1.234e12");
    assert_eq!(format_numeric_statistic(1.2346e12, 4), "1.235e12");
}
