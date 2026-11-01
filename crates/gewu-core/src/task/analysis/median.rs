//! Exact native order statistics. Only source IDs are materialized; no widened
//! column or full sort is needed. Selection checks cancellation during partitions.

use std::cmp::Ordering;

use arrow::{
    array::{
        downcast_integer_array, make_comparator, Array, ArrayRef, AsArray, DynComparator,
        Time32SecondArray, TimestampSecondArray,
    },
    compute::SortOptions,
    datatypes::*,
};

use super::{numeric::Numeric, AnalysisContext};
use crate::{column::format, scope::Selection, Error, Result};

pub(super) fn label(
    array: &ArrayRef,
    rows: &Selection,
    context: &AnalysisContext,
) -> Result<Option<String>> {
    let value = Numeric::new(array)
        .expect("validated numeric statistics")
        .accessor();
    let mut ids = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        context.check_at(i)?;
        if value(row).is_some_and(|v| !v.is_nan()) {
            ids.push(row as u32);
        }
    }
    if ids.is_empty() {
        return Ok(None);
    }
    let compare = make_comparator(array.as_ref(), array.as_ref(), SortOptions::default())?;
    let middle = ids.len() / 2;
    select(&mut ids, middle, &compare, &mut || context.check())?;
    let high = ids[middle] as usize;
    if ids.len() % 2 == 1 {
        return Ok(format::cell(
            array.as_ref(),
            &format::formatter(array.as_ref())?,
            high,
        ));
    }
    let mut low = ids[0] as usize;
    for (i, &row) in ids[..middle].iter().enumerate() {
        context.check_at(i)?;
        if compare(low, row as usize).is_lt() {
            low = row as usize;
        }
    }
    midpoint(array, low, high)
}

// Median-of-three fast path, with a linear-work budget before switching to
// median-of-medians pivots. The fallback prevents adversarial quadratic scans.
fn select(
    ids: &mut [u32],
    nth: usize,
    cmp: &DynComparator,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut range = ids;
    let mut target = nth;
    let mut budget = range.len().saturating_mul(3);
    loop {
        check()?;
        if range.len() <= 16 {
            range.sort_unstable_by(|a, b| cmp(*a as usize, *b as usize));
            return Ok(());
        }
        let pivot = if budget >= range.len() {
            budget -= range.len();
            let mut candidates = [range[0], range[range.len() / 2], range[range.len() - 1]];
            candidates.sort_unstable_by(|a, b| cmp(*a as usize, *b as usize));
            candidates[1]
        } else {
            // Move the median of each bounded group into the prefix, then
            // select that prefix's median recursively in the same ID buffer.
            let groups = range.len().div_ceil(5);
            for group in 0..groups {
                if group.is_multiple_of(512) {
                    check()?;
                }
                let start = group * 5;
                let end = (start + 5).min(range.len());
                range[start..end].sort_unstable_by(|a, b| cmp(*a as usize, *b as usize));
                range.swap(group, start + (end - start) / 2);
            }
            select(&mut range[..groups], groups / 2, cmp, check)?;
            range[groups / 2]
        };
        let (mut lower, mut current, mut upper) = (0, 0, range.len());
        let mut work = 0usize;
        while current < upper {
            if work.is_multiple_of(4096) {
                check()?;
            }
            work += 1;
            match cmp(range[current] as usize, pivot as usize) {
                Ordering::Less => {
                    range.swap(lower, current);
                    lower += 1;
                    current += 1;
                }
                Ordering::Equal => current += 1,
                Ordering::Greater => {
                    upper -= 1;
                    range.swap(current, upper);
                }
            }
        }
        if target < lower {
            range = &mut range[..lower];
        } else if target >= upper {
            range = &mut range[upper..];
            target -= upper;
        } else {
            return Ok(());
        }
    }
}

fn midpoint(array: &ArrayRef, low: usize, high: usize) -> Result<Option<String>> {
    use DataType::*;
    let decimal = |a, b, scale| scaled_midpoint(a, b, scale).map(Some);
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
            let value = Numeric::new(array).unwrap().accessor();
            let (a, b) = (value(low).unwrap(), value(high).unwrap());
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
fn scaled_midpoint(a: i256, b: i256, mut scale: i32) -> Result<String> {
    let sum = a.checked_add(b).ok_or_else(|| {
        Error::InvalidArgument("decimal midpoint exceeds coefficient range".into())
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
        return Ok(scaled_midpoint(a.into(), b.into(), scale(unit))? + " s");
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
    let seconds = i64::try_from(seconds)
        .map_err(|_| Error::InvalidArgument("temporal midpoint exceeds seconds range".into()))?;
    let scalar: ArrayRef = if matches!(data_type, DataType::Time32(_) | DataType::Time64(_)) {
        let seconds = i32::try_from(seconds)
            .map_err(|_| Error::InvalidArgument("time midpoint exceeds seconds range".into()))?;
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
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        Date32Array, Decimal128Array, Decimal256Array, Float64Array, Int64Array,
        TimestampNanosecondArray, UInt64Array,
    };

    use super::*;

    fn median(array: ArrayRef) -> Option<String> {
        label(
            &array,
            &Selection::All(array.len()),
            &AnalysisContext::default(),
        )
        .unwrap()
    }

    #[test]
    fn medians_keep_native_precision_and_fractional_midpoints() {
        assert_eq!(
            median(Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX]))).as_deref(),
            Some("18446744073709551614.5")
        );
        assert_eq!(
            median(Arc::new(Int64Array::from(vec![Some(-1), None, Some(0)]))).as_deref(),
            Some("-0.5")
        );
        assert_eq!(
            median(Arc::new(Int64Array::from(vec![i64::MIN, i64::MAX]))).as_deref(),
            Some("-0.5")
        );
        assert_eq!(
            median(Arc::new(
                Decimal128Array::from(vec![123, 124])
                    .with_precision_and_scale(10, 2)
                    .unwrap()
            ))
            .as_deref(),
            Some("1.235")
        );
        assert_eq!(
            median(Arc::new(
                Decimal128Array::from(vec![0, 1])
                    .with_precision_and_scale(10, -2)
                    .unwrap()
            ))
            .as_deref(),
            Some("50")
        );
        let large: i256 =
            "9999999999999999999999999999999999999999999999999999999999999999999999999999"
                .parse()
                .unwrap();
        assert_eq!(
            median(Arc::new(
                Decimal256Array::from(vec![large - i256::ONE, large])
                    .with_precision_and_scale(76, 0)
                    .unwrap()
            ))
            .unwrap(),
            format!("{}.5", large - i256::ONE)
        );
        assert_eq!(
            median(Arc::new(Float64Array::from(vec![None, Some(f64::NAN)]))),
            None
        );
        assert_eq!(
            median(Arc::new(Float64Array::from(vec![
                f64::NEG_INFINITY,
                f64::INFINITY
            ]))),
            None
        );
        assert_eq!(
            median(Arc::new(Float64Array::from(vec![f64::MAX, f64::MAX]))),
            Some(f64::MAX.to_string())
        );
        assert_eq!(
            median(Arc::new(Float64Array::from(vec![0.0, f64::INFINITY]))).as_deref(),
            Some("inf")
        );
        let tiny = f64::from_bits(1);
        for (a, b, expected) in [(tiny, 2.0 * tiny, 2.0 * tiny), (-tiny, 2.0 * tiny, 0.0)] {
            assert_eq!(
                median(Arc::new(Float64Array::from(vec![a, b]))),
                Some(expected.to_string())
            );
        }
    }

    #[test]
    fn temporal_medians_keep_fractional_units_and_timezone() {
        assert_eq!(
            median(Arc::new(Date32Array::from(vec![0, 1]))).as_deref(),
            Some("1970-01-01T12:00:00")
        );
        assert_eq!(
            median(Arc::new(
                TimestampNanosecondArray::from(vec![0, 1]).with_timezone("UTC")
            ))
            .as_deref(),
            Some("1970-01-01T00:00:00.0000000005Z")
        );
        assert_eq!(
            median(Arc::new(
                TimestampNanosecondArray::from(vec![-1, 0]).with_timezone("UTC")
            ))
            .as_deref(),
            Some("1969-12-31T23:59:59.9999999995Z")
        );
        assert_eq!(
            median(Arc::new(
                TimestampSecondArray::from(vec![0, 1]).with_timezone("Europe/London")
            ))
            .as_deref(),
            Some("1970-01-01T01:00:00.5+01:00")
        );
    }

    #[test]
    fn selection_matches_sorted_reference_and_checks_cancellation() {
        for values in [
            (0..10003)
                .map(|i| ((i * 104729) % 10003) as i64)
                .collect::<Vec<_>>(),
            (0..10003).rev().map(i64::from).collect(),
            (0..10003).map(|i| i64::from(i % 7)).collect(),
        ] {
            let array: ArrayRef = Arc::new(Int64Array::from(values.clone()));
            let cmp =
                make_comparator(array.as_ref(), array.as_ref(), SortOptions::default()).unwrap();
            let mut reference = values.clone();
            reference.sort_unstable();
            for target in [0, 1, values.len() / 2, values.len() - 1] {
                let mut ids = (0..values.len() as u32).collect::<Vec<_>>();
                select(&mut ids, target, &cmp, &mut || Ok(())).unwrap();
                assert_eq!(values[ids[target] as usize], reference[target]);
                assert!(ids[..target]
                    .iter()
                    .all(|id| values[*id as usize] <= reference[target]));
            }
            let mut checks = 0;
            let mut ids = (0..values.len() as u32).collect::<Vec<_>>();
            assert!(matches!(
                select(&mut ids, values.len() / 2, &cmp, &mut || {
                    checks += 1;
                    if checks == 3 {
                        Err(Error::Cancelled)
                    } else {
                        Ok(())
                    }
                }),
                Err(Error::Cancelled)
            ));
        }
        let array: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 100, 101]));
        assert_eq!(
            label(
                &array,
                &Selection::Rows(Arc::new(vec![3, 1])),
                &AnalysisContext::default()
            )
            .unwrap()
            .as_deref(),
            Some("51.5")
        );
        let context = AnalysisContext::default();
        context.cancel();
        assert!(matches!(
            label(&array, &Selection::All(4), &context),
            Err(Error::Cancelled)
        ));
    }
}
