//! Floating search precision and exact equality are separate operations.

use arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::{DataType, Float16Type, Float32Type, Float64Type},
};

use crate::{predicate::Predicate, Error, Result};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    let value: f64 = text.parse().ok()?;
    if value.is_nan() {
        return Some(nan(array));
    }
    if value.is_infinite()
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        )
    {
        return None;
    }
    if value.is_infinite() {
        return Some(bound_range(array, value, value));
    }
    let (lo, hi) = search_bounds(text)?;
    Some(bound_range(array, lo, hi))
}

fn search_bounds(text: &str) -> Option<(f64, f64)> {
    let value: f64 = text.parse().ok().filter(|v: &f64| v.is_finite())?;
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i64>().ok()?),
        None => (text, 0),
    };
    let decimals = mantissa.split_once('.').map_or(0, |(_, d)| d.len() as i64);
    // Saturate only the exponent calculation, never the target. Very tiny
    // precision produces the exact parsed value, including at zero.
    let power = exponent
        .saturating_sub(decimals)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let half = 0.5 * 10f64.powi(power);
    Some((
        (value - half).min(value),
        (value + half).next_down().max(value),
    ))
}

pub(in crate::column) fn bound_range(array: &ArrayRef, lo: f64, hi: f64) -> Predicate<'_> {
    macro_rules! native {
        ($ty:ty) => {{
            let array = array.as_primitive::<$ty>();
            Predicate::rows(Box::new(move |row| {
                if array.is_null(row) {
                    return false;
                }
                let value = f64::from(array.value(row));
                value >= lo && value <= hi
            }))
        }};
    }
    match array.data_type() {
        DataType::Float16 => native!(Float16Type),
        DataType::Float32 => native!(Float32Type),
        DataType::Float64 => native!(Float64Type),
        _ => unreachable!("float dispatch"),
    }
}

pub(super) fn nan(array: &ArrayRef) -> Predicate<'_> {
    macro_rules! native {
        ($ty:ty) => {{
            let a = array.as_primitive::<$ty>();
            Predicate::rows(Box::new(move |r| a.is_valid(r) && a.value(r).is_nan()))
        }};
    }
    match array.data_type() {
        DataType::Float16 => native!(Float16Type),
        DataType::Float32 => native!(Float32Type),
        DataType::Float64 => native!(Float64Type),
        _ => unreachable!("float dispatch"),
    }
}

pub(super) fn equals<'a>(array: &'a ArrayRef, text: &str) -> Result<Predicate<'a>> {
    let target = parse(text)?;
    if target.is_infinite()
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        )
    {
        return Ok(Predicate::rows(Box::new(|_| false)));
    }
    if target.is_nan() {
        return Ok(nan(array));
    }
    // Equality parses to the native representation, unlike a search interval
    // or an inclusive range endpoint. No stored array is cast.
    let original = target;
    let target = match array.data_type() {
        DataType::Float16 => {
            use arrow::compute::kernels::cast_utils::Parser;
            let value = Float16Type::parse(text).ok_or_else(|| {
                Error::InvalidArgument(format!("cannot read {text:?} as Float16"))
            })?;
            f64::from(value)
        }
        DataType::Float32 => f64::from(
            text.parse::<f32>()
                .map_err(|_| Error::InvalidArgument(format!("cannot read {text:?} as Float32")))?,
        ),
        DataType::Float64 => target,
        _ => unreachable!("float dispatch"),
    };
    if original.is_finite() && !target.is_finite() {
        return Ok(Predicate::rows(Box::new(|_| false)));
    }
    Ok(bound_range(array, target, target))
}

pub(in crate::column) fn integer_endpoint(text: &str, rounded: f64, lower: bool) -> f64 {
    let integer = text
        .parse::<i64>()
        .map(i128::from)
        .or_else(|_| text.parse::<u64>().map(i128::from));
    let Ok(integer) = integer else {
        return rounded;
    };
    integer_value_endpoint(integer, rounded, lower)
}

pub(in crate::column) fn integer_value_endpoint(integer: i128, rounded: f64, lower: bool) -> f64 {
    if lower && (rounded as i128) < integer {
        rounded.next_up()
    } else if !lower && (rounded as i128) > integer {
        rounded.next_down()
    } else {
        rounded
    }
}

fn parse(text: &str) -> Result<f64> {
    text.parse().map_err(|_| {
        Error::InvalidArgument(format!("cannot read {text:?} as a floating-point number"))
    })
}

#[cfg(test)]
mod tests {
    use super::{equals, search_bounds};

    #[test]
    fn equality_parses_float32_without_double_rounding() {
        use std::sync::Arc;

        use arrow::array::{ArrayRef, Float32Array};

        use crate::engine::CancelToken;

        // Just above the midpoint between 1.0_f32 and its successor. Parsing
        // through f64 rounds to the midpoint, then rounds the tie downward.
        let text = "1.0000000596046447753906250000001";
        assert_eq!(text.parse::<f64>().unwrap() as f32, 1.0);
        assert_eq!(text.parse::<f32>().unwrap(), 1.0_f32.next_up());
        let array: ArrayRef = Arc::new(Float32Array::from(vec![1.0, 1.0_f32.next_up()]));
        let mut matched = [false; 2];
        equals(&array, text)
            .unwrap()
            .evaluate(0..2, &mut matched, &CancelToken::never())
            .unwrap();
        assert_eq!(matched, [false, true]);
    }

    fn matches(query: &str, value: f64) -> bool {
        let (lo, hi) = search_bounds(query).unwrap();
        value >= lo && value <= hi
    }

    #[test]
    fn search_preserves_target_when_interval_collapses() {
        for query in ["3089.62000000000000", "-3089.62000000000000"] {
            let value: f64 = query.parse().unwrap();
            assert!(matches(query, value));
            assert!(!matches(query, value.next_down()));
            assert!(!matches(query, value.next_up()));
        }
        let value = 4_503_599_627_370_496.0_f64;
        assert!(matches("4503599627370496", value));
        assert!(matches("4503599627370496", value.next_down()));
        assert!(!matches("4503599627370496", value.next_up()));
        assert!(matches("0e-324", 0.0));
        assert!(matches("0e-324", -0.0));
        assert!(!matches("0e-324", 0.0_f64.next_up()));
        assert!(!matches("0e-324", 0.0_f64.next_down()));
    }

    #[test]
    fn search_preserves_precision_and_half_open_boundaries() {
        assert!(matches("1.2", 1.23));
        assert!(!matches("1.20", 1.23));
        for (query, lo, hi) in [("2", 1.5_f64, 2.5_f64), ("-2", -2.5, -1.5)] {
            assert!(!matches(query, lo.next_down()));
            assert!(matches(query, lo));
            assert!(matches(query, hi.next_down()));
            assert!(!matches(query, hi));
        }
    }
}
