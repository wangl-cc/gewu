use std::sync::Arc;

use arrow::{
    array::{Decimal128Array, Decimal256Array},
    compute::cast,
    datatypes::{Field, Fields},
};

use super::*;

#[test]
fn decimal_scalar_conversion_matches_arrows_scale_and_rounding() {
    for scale in [-38, -2, 0, 2, 38] {
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![
                Some(1),
                Some(-1),
                Some(9_007_199_254_740_993),
                Some(10_i128.pow(38) - 1),
                None,
            ])
            .with_precision_and_scale(38, scale)
            .unwrap(),
        );
        check_decimal_conversion(array);
    }
    for scale in [-76, -2, 0, 2, 76] {
        let large: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let array: ArrayRef = Arc::new(
            Decimal256Array::from(vec![
                Some(i256::from(1)),
                Some(i256::from(-1)),
                Some(large),
                Some(-large),
                None,
            ])
            .with_precision_and_scale(76, scale)
            .unwrap(),
        );
        check_decimal_conversion(array);
    }
}

fn check_decimal_conversion(array: ArrayRef) {
    let converted = cast(&array, &DataType::Float64).unwrap();
    let expected = converted.as_primitive::<Float64Type>();
    for row in 0..array.len() {
        if array.is_null(row) {
            continue;
        }
        let actual = match array.data_type() {
            DataType::Decimal128(_, scale) => Decimal128Type::as_float(
                array.as_primitive::<Decimal128Type>().value(row),
                i32::from(*scale),
            ),
            DataType::Decimal256(_, scale) => Decimal256Type::as_float(
                array.as_primitive::<Decimal256Type>().value(row),
                i32::from(*scale),
            ),
            _ => unreachable!("test decimal types"),
        };
        assert_eq!(
            actual.to_bits(),
            expected.value(row).to_bits(),
            "{:?} row {row}",
            array.data_type()
        );
    }
}

fn every_type() -> Vec<DataType> {
    use DataType::*;
    let item = Arc::new(Field::new("item", Int32, true));
    let mut types = vec![
        Null,
        Boolean,
        Int8,
        Int16,
        Int32,
        Int64,
        UInt8,
        UInt16,
        UInt32,
        UInt64,
        Float16,
        Float32,
        Float64,
        Date32,
        Date64,
        Interval(arrow::datatypes::IntervalUnit::YearMonth),
        Interval(arrow::datatypes::IntervalUnit::DayTime),
        Interval(arrow::datatypes::IntervalUnit::MonthDayNano),
        Binary,
        LargeBinary,
        BinaryView,
        FixedSizeBinary(4),
        Utf8,
        LargeUtf8,
        Utf8View,
        List(item.clone()),
        LargeList(item.clone()),
        FixedSizeList(item, 2),
        Decimal32(9, 2),
        Decimal64(18, 2),
        Decimal128(38, 2),
        Decimal256(76, 2),
        Dictionary(Box::new(Int32), Box::new(Utf8)),
        Dictionary(Box::new(Int32), Box::new(Int64)),
        Struct(Fields::from(vec![Field::new("f", Int32, true)])),
    ];
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        types.push(Timestamp(unit, None));
        types.push(Timestamp(unit, Some("UTC".into())));
        types.push(Duration(unit));
    }
    types.push(Time32(TimeUnit::Second));
    types.push(Time32(TimeUnit::Millisecond));
    types.push(Time64(TimeUnit::Microsecond));
    types.push(Time64(TimeUnit::Nanosecond));
    types
}

#[test]
fn temporal_mean_labels_round_physical_units_and_exclude_invalid_means() {
    assert_eq!(
        temporal_mean_label(&DataType::Date32, Some(1.4))
            .unwrap()
            .as_deref(),
        Some("1970-01-02")
    );
    assert_eq!(
        temporal_mean_label(&DataType::Time32(TimeUnit::Millisecond), Some(1.5))
            .unwrap()
            .as_deref(),
        Some("00:00:00.002")
    );
    for mean in [
        None,
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(i64::MAX as f64),
    ] {
        assert_eq!(
            temporal_mean_label(&DataType::Timestamp(TimeUnit::Nanosecond, None), mean).unwrap(),
            None
        );
    }
    assert_eq!(
        temporal_mean_label(&DataType::Int64, Some(1.0)).unwrap(),
        None
    );
}

#[test]
fn supported_types_have_native_dispatch() {
    for data_type in every_type() {
        let array = arrow::array::new_null_array(&data_type, 1);
        if let Some(numeric) = Numeric::new(&array) {
            let mut out = [0.0];
            let all = Selection::Range(0..1);
            numeric.write_f64(&all, &mut out);
            assert!(out[0].is_nan(), "{data_type}");
            assert!(numeric.median_values(&all).is_none(), "{data_type}");
        }
    }
}
