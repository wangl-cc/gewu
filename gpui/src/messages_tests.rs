use super::{counted, format_count};

#[test]
fn large_display_counts_keep_every_digit() {
    assert_eq!(format_count(0), "0");
    assert_eq!(format_count(2_000_000), "2,000,000");
    assert_eq!(format_count(u64::MAX), "18,446,744,073,709,551,615");
    assert_eq!(counted(1, "row"), "1 row");
    assert_eq!(counted(50_000, "row"), "50,000 rows");
}
