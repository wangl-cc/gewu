//! Exact native order statistics. Only the non-null native values are copied
//! for selection; no widened column or full sort is needed.
//!
//! Nulls and NaNs are excluded. An even-sized median averages the two central
//! values: integers and decimals stay exact, with one extra decimal place when
//! needed; floating-point midpoints avoid overflow, infinities take part in
//! ordering, and an opposite-infinity midpoint is undefined. Temporal
//! midpoints keep fractional source units, so a date midpoint can include a
//! time. Duration midpoints use decimal seconds with an `s` suffix.

use arrow::{
    array::{
        downcast_integer_array, Array, ArrayRef, AsArray, Time32SecondArray, TimestampSecondArray,
    },
    datatypes::*,
};

use super::numeric::Numeric;
use crate::{col_view::Selection, format, stats::Result};

pub(crate) fn label(numeric: Numeric<'_>, selection: &Selection<'_>) -> Result<Option<String>> {
    let Some(central) = numeric.median_values(selection) else {
        return Ok(None);
    };
    if central.len() == 1 {
        return Ok(format::cell(
            central.as_ref(),
            &format::formatter(central.as_ref())?,
            0,
        ));
    }
    midpoint(&central, 0, 1)
}

fn midpoint(array: &ArrayRef, low: usize, high: usize) -> Result<Option<String>> {
    use DataType::*;
    let decimal = |a, b, scale| scaled_midpoint(a, b, scale, array.data_type()).map(Some);
    let label = match array.data_type() {
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            return downcast_integer_array!(array => {
                decimal(i256::from_i128(array.value(low) as i128), i256::from_i128(array.value(high) as i128), 0)
            }, _ => unreachable!("integer match"));
        }
        Decimal32(_, scale) => {
            let a = array.as_primitive::<Decimal32Type>();
            return decimal(a.value(low).into(), a.value(high).into(), *scale as i32);
        }
        Decimal64(_, scale) => {
            let a = array.as_primitive::<Decimal64Type>();
            return decimal(a.value(low).into(), a.value(high).into(), *scale as i32);
        }
        Decimal128(_, scale) => {
            let a = array.as_primitive::<Decimal128Type>();
            return decimal(a.value(low).into(), a.value(high).into(), *scale as i32);
        }
        Decimal256(_, scale) => {
            let a = array.as_primitive::<Decimal256Type>();
            return decimal(a.value(low), a.value(high), *scale as i32);
        }
        Float16 | Float32 | Float64 => {
            let numeric = Numeric::new(array).unwrap();
            let [mut a, mut b] = [0.0; 2];
            numeric.write_f64(
                &Selection::Range(low..low + 1),
                std::slice::from_mut(&mut a),
            );
            numeric.write_f64(
                &Selection::Range(high..high + 1),
                std::slice::from_mut(&mut b),
            );
            let result = a.midpoint(b);
            return Ok((!result.is_nan()).then(|| result.to_string()));
        }
        _ => {
            let (a, b) = temporal_pair(array, low, high);
            temporal_midpoint(array.data_type(), a, b)?
        }
    };
    Ok(Some(label))
}

/// Decimal text of (a+b)/2 * 10^-scale; preserves the extra midpoint digit.
fn scaled_midpoint(a: i256, b: i256, mut scale: i32, data_type: &DataType) -> Result<String> {
    let sum = a
        .checked_add(b)
        .ok_or_else(|| crate::AnalysisError::ValueOutOfRange {
            operation: crate::AnalysisOperation::Statistics,
            data_type: data_type.clone(),
        })?;
    let two = i256::from(2);
    let half = sum / two;
    let mut digits = half.to_string().trim_start_matches('-').to_owned();
    if sum % two != i256::ZERO {
        if digits == "0" {
            digits.clear();
        }
        digits.push('5');
        scale += 1;
    }
    if scale < 0 {
        digits.extend(std::iter::repeat_n('0', (-scale) as usize));
    } else if scale > 0 {
        let scale = scale as usize;
        if digits.len() <= scale {
            digits = "0".repeat(scale + 1 - digits.len()) + &digits;
        }
        digits.insert(digits.len() - scale, '.');
    }
    if sum < i256::ZERO {
        digits.insert(0, '-');
    }
    Ok(digits)
}

fn temporal_pair(array: &ArrayRef, low: usize, high: usize) -> (i128, i128) {
    macro_rules! pair {
        ($ty:ty) => {{
            let a = array.as_primitive::<$ty>();
            (i128::from(a.value(low)), i128::from(a.value(high)))
        }};
    }
    match array.data_type() {
        DataType::Date32 => pair!(Date32Type),
        DataType::Date64 => pair!(Date64Type),
        DataType::Time32(TimeUnit::Second) => pair!(Time32SecondType),
        DataType::Time32(TimeUnit::Millisecond) => pair!(Time32MillisecondType),
        DataType::Time64(TimeUnit::Microsecond) => pair!(Time64MicrosecondType),
        DataType::Time64(TimeUnit::Nanosecond) => pair!(Time64NanosecondType),
        DataType::Timestamp(TimeUnit::Second, _) => pair!(TimestampSecondType),
        DataType::Timestamp(TimeUnit::Millisecond, _) => pair!(TimestampMillisecondType),
        DataType::Timestamp(TimeUnit::Microsecond, _) => pair!(TimestampMicrosecondType),
        DataType::Timestamp(TimeUnit::Nanosecond, _) => pair!(TimestampNanosecondType),
        DataType::Duration(TimeUnit::Second) => pair!(DurationSecondType),
        DataType::Duration(TimeUnit::Millisecond) => pair!(DurationMillisecondType),
        DataType::Duration(TimeUnit::Microsecond) => pair!(DurationMicrosecondType),
        DataType::Duration(TimeUnit::Nanosecond) => pair!(DurationNanosecondType),
        _ => unreachable!("validated native numeric type"),
    }
}

fn temporal_midpoint(data_type: &DataType, a: i128, b: i128) -> Result<String> {
    let scale = |unit: &TimeUnit| match unit {
        TimeUnit::Second => 0,
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
    };
    if let DataType::Duration(unit) = data_type {
        return Ok(scaled_midpoint(a.into(), b.into(), scale(unit), data_type)? + " s");
    }
    let ticks_per_second = match data_type {
        DataType::Date32 => 1,
        DataType::Date64 => 1000,
        DataType::Time32(unit) | DataType::Time64(unit) | DataType::Timestamp(unit, _) => {
            10_i128.pow(scale(unit) as u32)
        }
        _ => unreachable!("temporal type"),
    };
    let sum = (a + b)
        * if *data_type == DataType::Date32 {
            86400
        } else {
            1
        };
    let denominator = ticks_per_second * 2;
    let seconds = sum.div_euclid(denominator);
    let fraction = sum.rem_euclid(denominator) * (10_000_000_000 / denominator);
    let out_of_range = || crate::AnalysisError::ValueOutOfRange {
        operation: crate::AnalysisOperation::Statistics,
        data_type: data_type.clone(),
    };
    let seconds = i64::try_from(seconds).map_err(|_| out_of_range())?;
    let scalar: ArrayRef = if matches!(data_type, DataType::Time32(_) | DataType::Time64(_)) {
        let seconds = i32::try_from(seconds).map_err(|_| out_of_range())?;
        std::sync::Arc::new(Time32SecondArray::from(vec![seconds]))
    } else {
        let timezone = if let DataType::Timestamp(_, timezone) = data_type {
            timezone.clone()
        } else {
            None
        };
        std::sync::Arc::new(TimestampSecondArray::from(vec![seconds]).with_timezone_opt(timezone))
    };
    let mut text = format::formatter(scalar.as_ref())?.value(0).to_string();
    if fraction > 0 {
        let fraction = format!("{fraction:010}");
        let fraction = fraction.trim_end_matches('0');
        let end = text.find('T').map_or(8, |at| at + 9);
        if text.is_char_boundary(end) {
            text.insert_str(end, &(".".to_owned() + fraction));
        }
    } else if matches!(data_type, DataType::Date32 | DataType::Date64)
        && seconds.rem_euclid(86400) == 0
    {
        if let Some(end) = text.find('T') {
            text.truncate(end);
        }
    }
    Ok(text)
}

#[cfg(test)]
#[path = "median_tests.rs"]
mod tests;
