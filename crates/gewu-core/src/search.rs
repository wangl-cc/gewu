//! Global search and the "contains" filter, without per-column indexes.
//!
//! What a query matches depends on the column type:
//!
//! - Text: case-insensitive substring, with Arrow's `like` kernels in place.
//! - Integers: the query read as an integer, compared by value.
//! - Floats and decimals: the query read as a number, matching the interval its precision implies:
//!   `3.14` matches [3.135, 3.145), `3` matches [2.5, 3.5), `1e3` matches [500, 1500). `nan` and
//!   `inf` match those.
//! - Dates and timestamps: a date prefix (`2024`, `2024-03`, `2024-03-15`, optionally with ` 10`, `
//!   10:30` or ` 10:30:15`) matches the whole period it names, in the column's own unit and time
//!   zone. Date columns accept only the date parts.
//! - Booleans: `true` or `false`.
//! - Anything else: substring of the displayed text, formatted into one reused buffer.
//!
//! A column the query cannot mean anything for (a word in a number column)
//! is skipped. Nothing is cached after the call.

use std::{fmt::Write, sync::Arc};

use arrow::{
    array::{
        downcast_integer_array, downcast_temporal_array, Array, ArrayRef, AsArray, BooleanArray,
        Datum, LargeStringArray, PrimitiveArray, Scalar, StringArray, StringViewArray,
    },
    compute::{
        cast,
        kernels::comparison::{ilike, like},
    },
    datatypes::{ArrowPrimitiveType, DataType, Float16Type, Float32Type, Float64Type},
};
use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::{engine::CancelToken, format, numeric::Numeric, Bound, Result};

/// How many rows to process between cancellation checks.
const CHUNK: usize = 64 * 1024;

/// Set `hits[r] = true` for rows that match `query` (see the module docs).
/// Nulls never match. Rows already marked stay marked.
pub(crate) fn mark_matches(
    array: &ArrayRef,
    query: &str,
    case_sensitive: bool,
    hits: &mut [bool],
    token: &CancelToken,
) -> Result<()> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(());
    }
    use DataType::*;
    match array.data_type() {
        Utf8 | LargeUtf8 | Utf8View => {
            let mask = string_kernel(array, query, case_sensitive)?;
            for (hit, m) in hits.iter_mut().zip(mask.iter()) {
                *hit |= m == Some(true);
            }
        }
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            let Ok(target) = query.trim_start_matches('+').parse::<i128>() else {
                return Ok(());
            };
            downcast_integer_array!(array => {
                mark_integer(array, target, hits, token)?;
            }
            _ => unreachable!("integer types were matched above"))
        }
        Float16 | Float32 | Float64 => {
            let Some(test) = float_test(query) else {
                return Ok(());
            };
            match array.data_type() {
                Float16 => mark_values(array.as_primitive::<Float16Type>(), hits, token, |v| {
                    test(f64::from(v))
                })?,
                Float32 => mark_values(array.as_primitive::<Float32Type>(), hits, token, |v| {
                    test(f64::from(v))
                })?,
                Float64 => mark_values(array.as_primitive::<Float64Type>(), hits, token, test)?,
                _ => unreachable!("float types were matched above"),
            }
        }
        Decimal128(..) | Decimal256(..) => {
            // Keep Arrow's scale-aware decimal conversion and rounding semantics.
            let Some(test) = float_test(query) else {
                return Ok(());
            };
            let values = cast(array, &Float64)?;
            let a = values.as_primitive::<Float64Type>();
            mark(hits, token, |r| a.is_valid(r) && test(a.value(r)))?;
        }
        Date32 | Date64 | Timestamp(..) => {
            let date_only = matches!(array.data_type(), Date32 | Date64);
            let Some((start, end)) = period(query, date_only) else {
                return Ok(());
            };
            let (Some(lo), Some(hi)) = (physical(array, &start)?, physical(array, &end)?) else {
                return Ok(());
            };
            downcast_temporal_array!(array => {
                mark_period(array, lo..hi, hits, token)?;
            }
            _ => unreachable!("date and timestamp types were matched above"))
        }
        Boolean => {
            let target = match query.to_ascii_lowercase().as_str() {
                "true" => true,
                "false" => false,
                _ => return Ok(()),
            };
            let a = array.as_boolean();
            mark(hits, token, |r| a.is_valid(r) && a.value(r) == target)?;
        }
        _ => formatted_substring(array, query, case_sensitive, hits, token)?,
    }
    Ok(())
}

/// Compare in the column's native type; out-of-range targets cannot match.
fn mark_integer<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    target: i128,
    hits: &mut [bool],
    token: &CancelToken,
) -> Result<()>
where
    T::Native: TryFrom<i128>,
{
    let Ok(target) = T::Native::try_from(target) else {
        return Ok(());
    };
    mark_values(array, hits, token, |v| v == target)
}

fn mark_period<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    period: std::ops::Range<i64>,
    hits: &mut [bool],
    token: &CancelToken,
) -> Result<()>
where
    T::Native: Into<i64>,
{
    mark_values(array, hits, token, |v| period.contains(&v.into()))
}

/// Borrow native values without materializing a widened array.
fn mark_values<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    hits: &mut [bool],
    token: &CancelToken,
    test: impl Fn(T::Native) -> bool,
) -> Result<()> {
    mark(hits, token, |r| array.is_valid(r) && test(array.value(r)))
}

/// Mark rows passing `test`, checking for cancellation between chunks.
fn mark(hits: &mut [bool], token: &CancelToken, test: impl Fn(usize) -> bool) -> Result<()> {
    let len = hits.len();
    for start in (0..len).step_by(CHUNK) {
        token.check()?;
        let end = (start + CHUNK).min(len);
        for (r, hit) in (start..end).zip(&mut hits[start..end]) {
            if !*hit && test(r) {
                *hit = true;
            }
        }
    }
    Ok(())
}

/// `like`/`ilike` with the query as a literal substring.
fn string_kernel(array: &ArrayRef, query: &str, case_sensitive: bool) -> Result<BooleanArray> {
    let pattern = format!("%{}%", escape_like(query));
    let scalar: Box<dyn Datum> = match array.data_type() {
        DataType::LargeUtf8 => Box::new(Scalar::new(LargeStringArray::from(vec![pattern]))),
        DataType::Utf8View => Box::new(Scalar::new(StringViewArray::from(vec![pattern]))),
        _ => Box::new(Scalar::new(StringArray::from(vec![pattern]))),
    };
    Ok(if case_sensitive {
        like(array, scalar.as_ref())?
    } else {
        ilike(array, scalar.as_ref())?
    })
}

/// Escape `%`, `_` and `\` so they match literally.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

type FloatTest = Box<dyn Fn(f64) -> bool>;

/// What a query means for a float column, or `None` if it is not a number.
fn float_test(query: &str) -> Option<FloatTest> {
    let lower = query.to_ascii_lowercase();
    match lower.as_str() {
        "nan" => return Some(Box::new(f64::is_nan)),
        "inf" | "+inf" | "infinity" | "+infinity" => return Some(Box::new(|v| v == f64::INFINITY)),
        "-inf" | "-infinity" => return Some(Box::new(|v| v == f64::NEG_INFINITY)),
        _ => {}
    }
    let value: f64 = query.parse().ok().filter(|v: &f64| v.is_finite())?;
    // Half a unit in the last typed digit of the mantissa.
    let (mantissa, exponent) = match lower.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (lower.as_str(), 0),
    };
    let decimals = mantissa.split_once('.').map_or(0, |(_, d)| d.len() as i32);
    let half = 0.5 * 10f64.powi(exponent - decimals);
    let (lo, hi) = (value - half, value + half);
    // Rounding can collapse the interval or its upper bound onto the target.
    // Always include the parsed value without widening the interval.
    Some(Box::new(move |v| v == value || (v >= lo && v < hi)))
}

/// A run of exactly `len` ASCII digits.
fn digits(s: &str, len: usize) -> Option<u32> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// The period a date/time prefix names, as `[start, end)` local times.
fn period(query: &str, date_only: bool) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let (date, time) = match query.split_once(['T', 't', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (query, None),
    };
    if date_only && time.is_some() {
        return None;
    }
    let parts: Vec<&str> = date.split('-').collect();
    let year = digits(parts.first()?, 4)? as i32;
    let midnight = |y: i32, m: u32, d: u32| NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(0, 0, 0);

    match (parts.len(), time) {
        (1, None) => Some((midnight(year, 1, 1)?, midnight(year + 1, 1, 1)?)),
        (2, None) => {
            let month = digits(parts[1], 2)?;
            let (ny, nm) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            Some((midnight(year, month, 1)?, midnight(ny, nm, 1)?))
        }
        (3, None) => {
            let start = midnight(year, digits(parts[1], 2)?, digits(parts[2], 2)?)?;
            Some((start, start + Duration::days(1)))
        }
        (3, Some(time)) => {
            let day = NaiveDate::from_ymd_opt(year, digits(parts[1], 2)?, digits(parts[2], 2)?)?;
            let t: Vec<&str> = time.split(':').collect();
            let field = |i: usize| t.get(i).map_or(Some(0), |s| digits(s, 2));
            let start = day.and_hms_opt(field(0)?, field(1)?, field(2)?)?;
            let step = match t.len() {
                1 => Duration::hours(1),
                2 => Duration::minutes(1),
                3 => Duration::seconds(1),
                _ => return None,
            };
            Some((start, start + step))
        }
        _ => None,
    }
}

/// A local date-time as the column's physical integer value, read the way
/// Arrow reads text for that type (so the time zone is the column's).
fn physical(array: &ArrayRef, at: &NaiveDateTime) -> Result<Option<i64>> {
    let text = match array.data_type() {
        DataType::Date32 | DataType::Date64 => at.date().to_string(),
        _ => at.format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let text: ArrayRef = Arc::new(StringArray::from(vec![text]));
    let typed = cast(&text, array.data_type())?;
    Ok(match Numeric::new(&typed).and_then(|n| n.bound(0)) {
        Some(Bound::Int(v)) => Some(v),
        _ => None,
    })
}

/// Substring of the displayed text, for types without a better rule.
fn formatted_substring(
    array: &ArrayRef,
    query: &str,
    case_sensitive: bool,
    hits: &mut [bool],
    token: &CancelToken,
) -> Result<()> {
    let lower_query = query.to_lowercase();
    let ascii = query.is_ascii();
    let f = format::formatter(array.as_ref())?;
    let mut buf = String::new();
    for start in (0..array.len()).step_by(CHUNK) {
        token.check()?;
        let end = (start + CHUNK).min(array.len());
        for (r, hit) in (start..end).zip(&mut hits[start..end]) {
            if *hit || array.is_null(r) {
                continue;
            }
            buf.clear();
            let _ = write!(buf, "{}", f.value(r));
            *hit = if case_sensitive {
                buf.contains(query)
            } else if ascii {
                contains_ignore_ascii_case(buf.as_bytes(), query.as_bytes())
            } else {
                buf.to_lowercase().contains(&lower_query)
            };
        }
    }
    Ok(())
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::float_test;

    #[test]
    fn float_search_preserves_target_when_interval_collapses() {
        for query in ["3089.62000000000000", "-3089.62000000000000"] {
            let value: f64 = query.parse().unwrap();
            let matches = float_test(query).unwrap();
            assert!(matches(value), "{query} must match its parsed value");
            assert!(
                !matches(value.next_down()),
                "{query} must exclude its predecessor"
            );
            assert!(
                !matches(value.next_up()),
                "{query} must exclude its successor"
            );
        }
    }

    #[test]
    fn float_search_preserves_target_when_only_upper_bound_rounds_to_it() {
        // At 2^52, the predecessor is 0.5 away and the successor is 1 away.
        let value = 4_503_599_627_370_496.0_f64;
        let matches = float_test("4503599627370496").unwrap();
        assert!(matches(value));
        assert!(matches(value.next_down()));
        assert!(!matches(value.next_up()));
    }

    #[test]
    fn float_search_preserves_zero_when_half_width_underflows() {
        let matches = float_test("0e-324").unwrap();
        assert!(matches(0.0));
        assert!(matches(-0.0));
        assert!(!matches(0.0_f64.next_down()));
        assert!(!matches(0.0_f64.next_up()));
    }

    #[test]
    fn float_search_preserves_typed_precision_and_half_open_boundaries() {
        assert!(float_test("1.2").unwrap()(1.23));
        assert!(!float_test("1.20").unwrap()(1.23));
        for (query, lo, hi) in [("2", 1.5_f64, 2.5_f64), ("-2", -2.5, -1.5)] {
            let matches = float_test(query).unwrap();
            assert!(!matches(lo.next_down()));
            assert!(matches(lo));
            assert!(matches(hi.next_down()));
            assert!(!matches(hi));
            assert!(!matches(f64::NAN));
            assert!(!matches(f64::INFINITY));
            assert!(!matches(f64::NEG_INFINITY));
        }
    }
}
