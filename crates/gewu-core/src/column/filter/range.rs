//! Range predicate compilation over Arrow arrays.

use arrow::array::ArrayRef;

use super::scalar::{
    decimal::Endpoint,
    float, integer, temporal,
};
use crate::{
    column::{bound::invalid_type, Bound},
    predicate::Predicate,
    types::{direct_domain, Domain},
    Error, Result,
};

pub(crate) fn prepare_range<'a>(
    array: &'a ArrayRef,
    min: Option<&Bound>,
    max: Option<&Bound>,
) -> Result<Predicate<'a>> {
    let data_type = array.data_type();
    for bound in [min, max].into_iter().flatten() {
        if bound.data_type() != data_type {
            return Err(Error::InvalidArgument(format!(
                "{} bound cannot filter {data_type}",
                bound.data_type()
            )));
        }
    }
    if min
        .zip(max)
        .is_some_and(|(lo, hi)| lo.compare(hi).is_gt())
    {
        return Err(Error::InvalidArgument(
            "minimum must not exceed maximum".into(),
        ));
    }
    if data_type.is_floating() {
        let lower = min.map_or(f64::NEG_INFINITY, |bound| bound.float_lower());
        let upper = max.map_or(f64::INFINITY, |bound| bound.float_upper());
        return Ok(float::bound_range(array, lower, upper));
    }
    let lower = min.map_or(Endpoint::Below, |bound| bound.exact_lower());
    let upper = max.map_or(Endpoint::Above, |bound| bound.exact_upper());
    Ok(match direct_domain(data_type) {
        Some(Domain::Integer) => integer::bind(array, lower, upper),
        Some(Domain::Decimal) => super::scalar::decimal::bind(array, lower, upper),
        Some(Domain::Date | Domain::Time | Domain::Timestamp | Domain::Duration) => {
            temporal::bind(array, lower, upper)
        }
        _ => return Err(invalid_type(data_type)),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            Decimal128Array, Decimal256Array, Float32Array, Float64Array, Int8Array,
            TimestampNanosecondArray, TimestampSecondArray, UInt8Array,
        },
        datatypes::{i256, DataType, TimeUnit},
    };

    use super::*;
    use crate::engine::CancelToken;

    fn matches(array: &ArrayRef, min: Option<&Bound>, max: Option<&Bound>) -> Vec<bool> {
        let mut out = vec![false; array.len()];
        prepare_range(array, min, max)
            .unwrap()
            .evaluate(0..array.len(), &mut out, &CancelToken::never())
            .unwrap();
        out
    }

    #[test]
    fn native_float_labels_roundtrip_through_exact_integer_text_parsing() {
        for value in [
            1.0000000000000001e18,
            -1.0000000000000001e18,
            u64::MAX as f64,
            f64::MAX,
            -f64::MAX,
            0.1,
            f64::MIN_POSITIVE,
        ] {
            let array: ArrayRef = Arc::new(Float64Array::from(vec![
                value.next_down(),
                value,
                value.next_up(),
            ]));
            let native = Bound::from_f64(array.data_type(), value).unwrap();
            let parsed = Bound::parse(array.data_type(), native.display()).unwrap();
            assert_eq!(matches(&array, Some(&parsed), Some(&parsed)), [
                false, true, false
            ]);
        }
    }

    #[test]
    fn native_constructors_validate_kind_without_narrowing_integer_bounds() {
        let array: ArrayRef = Arc::new(Int8Array::from(vec![-128, 0, 127]));
        let bound = Bound::from_i64(&DataType::Int8, 128).unwrap();
        assert_eq!(matches(&array, Some(&bound), None), [false; 3]);
        let array: ArrayRef = Arc::new(UInt8Array::from(vec![0, 255]));
        let below = Bound::from_i64(&DataType::UInt8, -1).unwrap();
        assert_eq!(matches(&array, Some(&below), None), [true; 2]);
        assert!(Bound::from_i64(&DataType::Boolean, 1).is_err());
        assert!(Bound::from_f64(&DataType::Int64, 1.0).is_err());
        assert!(Bound::from_f64(&DataType::Float64, f64::NAN).is_err());
        assert!(Bound::from_decimal(&DataType::Int64, i256::ONE, 0).is_err());
        assert!(Bound::parse(&DataType::Float64, "NaN").is_err());
    }

    #[test]
    fn decimal256_bound_roundtrips_and_execution_matches_reparsed() {
        let value: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let data_type = DataType::Decimal256(76, 2);
        let array: ArrayRef = Arc::new(
            Decimal256Array::from(vec![value, value + i256::ONE])
                .with_precision_and_scale(76, 2)
                .unwrap(),
        );
        let bound = Bound::from_decimal(&data_type, value, 2).unwrap();
        let reparsed = Bound::parse(&data_type, bound.display()).unwrap();
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [true, false]);
        assert_eq!(matches(&array, Some(&reparsed), Some(&reparsed)), [
            true, false
        ]);
        assert_eq!(bound.data_type(), &data_type);

        let data_type = DataType::Decimal128(20, -2);
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![12, 13])
                .with_precision_and_scale(20, -2)
                .unwrap(),
        );
        let bound = Bound::from_decimal(&data_type, i256::from(12), -2).unwrap();
        assert_eq!(bound.display(), "1200");
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [true, false]);
    }

    #[test]
    fn bounds_are_tied_to_full_logical_type() {
        let array: ArrayRef = Arc::new(Int8Array::from(vec![1]));
        let wider = Bound::from_i64(&DataType::Int64, 1).unwrap();
        assert!(prepare_range(&array, Some(&wider), None).is_err());
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![100])
                .with_precision_and_scale(20, 2)
                .unwrap(),
        );
        let other_scale =
            Bound::from_decimal(&DataType::Decimal128(20, 3), i256::from(1), 0).unwrap();
        assert!(prepare_range(&array, Some(&other_scale), None).is_err());
        let array: ArrayRef = Arc::new(TimestampSecondArray::from(vec![0]).with_timezone("UTC"));
        for data_type in [
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            DataType::Timestamp(TimeUnit::Second, Some("Europe/London".into())),
        ] {
            let other = Bound::from_i64(&data_type, 0).unwrap();
            assert!(prepare_range(&array, Some(&other), None).is_err());
        }
    }

    #[test]
    fn native_float_bounds_preserve_integer_endpoints_and_between_value_cuts() {
        let top = u64::MAX as f64;
        let array: ArrayRef = Arc::new(Float64Array::from(vec![top.next_down(), top]));
        let bound = Bound::from_u64(&DataType::Float64, u64::MAX).unwrap();
        assert_eq!(matches(&array, None, Some(&bound)), [true, false]);
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [false; 2]);
        let midpoint = (f64::from(1.0_f32) + f64::from(1.0_f32.next_up())) / 2.0;
        let array: ArrayRef = Arc::new(Float32Array::from(vec![1.0, 1.0_f32.next_up()]));
        let cut = Bound::from_f64(&DataType::Float32, midpoint).unwrap();
        assert_eq!(cut.axis_value(), midpoint);
        assert_eq!(matches(&array, Some(&cut), None), [false, true]);
        assert_eq!(matches(&array, None, Some(&cut)), [true, false]);
    }

    #[test]
    fn temporal_native_bound_formats_and_roundtrips_without_losing_nanoseconds() {
        let data_type = DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into()));
        let value = 9_007_199_254_740_993;
        let array: ArrayRef =
            Arc::new(TimestampNanosecondArray::from(vec![value - 1, value]).with_timezone("UTC"));
        let native = Bound::from_i64(&data_type, value).unwrap();
        let parsed = Bound::parse(&data_type, native.display()).unwrap();
        assert_eq!(native.axis_value(), (value - 1) as f64);
        assert_eq!(matches(&array, Some(&native), Some(&native)), [false, true]);
        assert_eq!(matches(&array, Some(&parsed), Some(&parsed)), [false, true]);
        assert_eq!(
            Bound::from_i64(&DataType::Date32, 1).unwrap().display(),
            "1970-01-02"
        );
    }

    #[test]
    fn reversed_bounds_are_rejected_before_axis_rounding() {
        let data_type = DataType::Decimal128(38, 0);
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![0])
                .with_precision_and_scale(38, 0)
                .unwrap(),
        );
        let lo = Bound::parse(&data_type, "9007199254740993.2").unwrap();
        let hi = Bound::parse(&data_type, "9007199254740993.1").unwrap();
        assert_eq!(lo.axis_value(), hi.axis_value());
        assert!(prepare_range(&array, Some(&lo), Some(&hi)).is_err());
        let array: ArrayRef = Arc::new(Float64Array::from(vec![f64::INFINITY]));
        let lo = Bound::from_f64(&DataType::Float64, f64::INFINITY).unwrap();
        let hi = Bound::parse(&DataType::Float64, "1e999").unwrap();
        assert!(prepare_range(&array, Some(&lo), Some(&hi)).is_err());
    }
}
