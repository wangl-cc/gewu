use arrow::datatypes::TimeUnit;

use super::*;

#[test]
fn parsed_coordinates_preserve_units_without_narrowing_input() {
    for (data_type, text, coordinate) in [
        (DataType::Int8, "1.2", 1.2),
        (DataType::Int8, "128", 128.0),
        (DataType::Float32, "0.1", 0.1),
        (DataType::Date32, "1970-01-02", 1.0),
        (DataType::Date32, "2024-01-02", 19724.0),
        (DataType::Time32(TimeUnit::Millisecond), "00:00:00.001", 1.0),
        (
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            "1970-01-01T00:00:00.000001Z",
            1.0,
        ),
        (
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            "2024-01-01T10:00:00Z",
            1704103200000000.0,
        ),
    ] {
        assert_eq!(
            Bound::parse(&data_type, text).unwrap().axis_value(),
            coordinate
        );
    }
}

#[test]
fn adjacent_exact_endpoints_can_share_only_their_plot_coordinate() {
    for (data_type, first, second) in [
        (
            DataType::UInt64,
            (u64::MAX - 1).to_string(),
            u64::MAX.to_string(),
        ),
        (
            DataType::Decimal128(16, 0),
            "9007199254740992".into(),
            "9007199254740993".into(),
        ),
        (
            DataType::Decimal256(16, 0),
            "9007199254740992".into(),
            "9007199254740993".into(),
        ),
    ] {
        let first = Bound::parse(&data_type, &first).unwrap();
        let second = Bound::parse(&data_type, &second).unwrap();
        assert_eq!(first.axis_value(), second.axis_value());
        assert_ne!(first.filter_text(), second.filter_text());
        assert_eq!(first.compare(&second), Ordering::Less);
    }
}
