//! What the search box means for each column.
//!
//! The search is not a matcher of its own: for each column it becomes one of
//! the ordinary [`ColumnTest`]s, picked by the column's [`DataKind`].
//!
//! - Text, lists, binary and other values: case-insensitive substring of the displayed text.
//! - Integers and durations: the query read as an integer, as a one-value range.
//! - Floats and decimals: the query read as a number, as the range its precision implies: `3.14` is
//!   [3.135, 3.145), `3` is [2.5, 3.5), `1e3` is [500, 1500). `nan` and `inf` match those.
//! - Dates and timestamps: a date prefix (`2024`, `2024-03`, `2024-03-15`, optionally with ` 10`, `
//!   10:30` or ` 10:30:15`) is the whole period it names, in the column's own unit and time zone.
//!   Date columns accept only the date parts.
//! - Times of day: `10`, `10:30` or `10:30:15`, as the period it names.
//! - Booleans: `true` or `false`.
//!
//! A column the query cannot mean anything for (a word in a number column)
//! gets no test and is skipped.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, StringArray},
    compute::cast,
    datatypes::DataType,
};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::{numeric::Numeric, predicate::ColumnTest, Bound, DataKind, Result};

/// The test the search `query` means for a column of `data_type`, if any.
pub(crate) fn search_test(data_type: &DataType, query: &str) -> Result<Option<ColumnTest>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(None);
    }
    Ok(match DataKind::of(data_type) {
        DataKind::Integer | DataKind::Duration => integer(query).map(|v| ColumnTest::Range {
            min: Some(v),
            max: Some(v),
        }),
        DataKind::Float | DataKind::Decimal => float(query),
        DataKind::Date | DataKind::Timestamp => {
            let date_only = DataKind::of(data_type) == DataKind::Date;
            match date_period(query, date_only) {
                Some((start, end)) => period_range(data_type, start, end)?,
                None => None,
            }
        }
        DataKind::Time => match time_period(query) {
            Some((start, step)) => {
                let start = NaiveDate::default().and_time(start);
                period_range(data_type, start, start + step)?
            }
            None => None,
        },
        DataKind::Boolean => match query.to_ascii_lowercase().as_str() {
            "true" => Some(ColumnTest::Bool(true)),
            "false" => Some(ColumnTest::Bool(false)),
            _ => None,
        },
        DataKind::Text
        | DataKind::Interval
        | DataKind::Binary
        | DataKind::List
        | DataKind::Other => Some(ColumnTest::Text {
            text: query.to_string(),
            case_sensitive: false,
        }),
    })
}

fn integer(query: &str) -> Option<Bound> {
    let q = query.strip_prefix('+').unwrap_or(query);
    q.parse::<i64>()
        .map(Bound::Int)
        .or_else(|_| q.parse::<u64>().map(Bound::UInt))
        .ok()
}

/// A number typed with some precision, as the range of values that round
/// to it.
fn float(query: &str) -> Option<ColumnTest> {
    let lower = query.to_ascii_lowercase();
    let exact = |v: f64| ColumnTest::Range {
        min: Some(Bound::Float(v)),
        max: Some(Bound::Float(v)),
    };
    match lower.as_str() {
        "nan" => return Some(ColumnTest::NaN),
        "inf" | "+inf" | "infinity" | "+infinity" => return Some(exact(f64::INFINITY)),
        "-inf" | "-infinity" => return Some(exact(f64::NEG_INFINITY)),
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
    // [value - half, value + half), made inclusive. Rounding can pull either
    // end onto the value itself; the value always matches.
    let lo = (value - half).min(value);
    let hi = (value + half).next_down().max(value);
    Some(ColumnTest::Range {
        min: Some(Bound::Float(lo)),
        max: Some(Bound::Float(hi)),
    })
}

/// A run of exactly `len` ASCII digits.
fn digits(s: &str, len: usize) -> Option<u32> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// The period a date/time prefix names, as `[start, end)` local times.
fn date_period(query: &str, date_only: bool) -> Option<(NaiveDateTime, NaiveDateTime)> {
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
            let (start, step) = time_period(time)?;
            let start = day.and_time(start);
            Some((start, start + step))
        }
        _ => None,
    }
}

/// `HH`, `HH:MM` or `HH:MM:SS`: when the period starts and how long it is.
fn time_period(query: &str) -> Option<(NaiveTime, Duration)> {
    let t: Vec<&str> = query.split(':').collect();
    let field = |i: usize| t.get(i).map_or(Some(0), |s| digits(s, 2));
    let start = NaiveTime::from_hms_opt(field(0)?, field(1)?, field(2)?)?;
    let step = match t.len() {
        1 => Duration::hours(1),
        2 => Duration::minutes(1),
        3 => Duration::seconds(1),
        _ => return None,
    };
    Some((start, step))
}

/// A `[start, end)` period as an inclusive range of the column's physical
/// values, read the way Arrow reads text for that type (so in the column's
/// unit and time zone). `None` when either end is out of the type's range.
fn period_range(
    data_type: &DataType,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Option<ColumnTest>> {
    let (Some(lo), Some(hi)) = (physical(data_type, &start)?, physical(data_type, &end)?) else {
        return Ok(None);
    };
    Ok(Some(ColumnTest::Range {
        min: Some(Bound::Int(lo)),
        max: Some(Bound::Int(hi - 1)),
    }))
}

/// A local date-time as the physical integer of a temporal type.
fn physical(data_type: &DataType, at: &NaiveDateTime) -> Result<Option<i64>> {
    let text = match data_type {
        DataType::Date32 | DataType::Date64 => at.date().to_string(),
        DataType::Time32(_) | DataType::Time64(_) => {
            if at.date() != NaiveDate::default() {
                // 24:00 is past the end of a time-of-day column; one past the
                // largest value is the day's length in the column's unit.
                return Ok(day_length(data_type));
            }
            at.format("%H:%M:%S").to_string()
        }
        _ => at.format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let text: ArrayRef = Arc::new(StringArray::from(vec![text]));
    let typed = cast(&text, data_type)?;
    Ok(match Numeric::new(&typed).and_then(|n| n.bound(0)) {
        Some(Bound::Int(v)) => Some(v),
        _ => None,
    })
}

fn day_length(data_type: &DataType) -> Option<i64> {
    use arrow::datatypes::TimeUnit::*;
    let seconds = 86_400i64;
    Some(match data_type {
        DataType::Time32(Second) => seconds,
        DataType::Time32(Millisecond) => seconds * 1_000,
        DataType::Time64(Microsecond) => seconds * 1_000_000,
        DataType::Time64(Nanosecond) => seconds * 1_000_000_000,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the search for `query` on a float column accepts `v`.
    fn matches(query: &str, v: f64) -> bool {
        match float(query).unwrap() {
            ColumnTest::Range {
                min: Some(Bound::Float(lo)),
                max: Some(Bound::Float(hi)),
            } => v >= lo && v <= hi,
            ColumnTest::NaN => v.is_nan(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn float_search_preserves_target_when_interval_collapses() {
        for query in ["3089.62000000000000", "-3089.62000000000000"] {
            let value: f64 = query.parse().unwrap();
            assert!(matches(query, value), "{query} must match its parsed value");
            assert!(!matches(query, value.next_down()));
            assert!(!matches(query, value.next_up()));
        }
    }

    #[test]
    fn float_search_preserves_target_when_only_upper_bound_rounds_to_it() {
        // At 2^52, the predecessor is 0.5 away and the successor is 1 away.
        let value = 4_503_599_627_370_496.0_f64;
        assert!(matches("4503599627370496", value));
        assert!(matches("4503599627370496", value.next_down()));
        assert!(!matches("4503599627370496", value.next_up()));
    }

    #[test]
    fn float_search_preserves_zero_when_half_width_underflows() {
        assert!(matches("0e-324", 0.0));
        assert!(matches("0e-324", -0.0));
        assert!(!matches("0e-324", 0.0_f64.next_down()));
        assert!(!matches("0e-324", 0.0_f64.next_up()));
    }

    #[test]
    fn float_search_preserves_typed_precision_and_half_open_boundaries() {
        assert!(matches("1.2", 1.23));
        assert!(!matches("1.20", 1.23));
        for (query, lo, hi) in [("2", 1.5_f64, 2.5_f64), ("-2", -2.5, -1.5)] {
            assert!(!matches(query, lo.next_down()));
            assert!(matches(query, lo));
            assert!(matches(query, hi.next_down()));
            assert!(!matches(query, hi));
            assert!(!matches(query, f64::NAN));
            assert!(!matches(query, f64::INFINITY));
            assert!(!matches(query, f64::NEG_INFINITY));
        }
        assert!(matches("nan", f64::NAN));
        assert!(matches("-inf", f64::NEG_INFINITY));
    }

    #[test]
    fn words_mean_nothing_to_numbers() {
        assert!(search_test(&DataType::Int64, "cat").unwrap().is_none());
        assert!(search_test(&DataType::Float64, "cat").unwrap().is_none());
        assert!(search_test(&DataType::Date32, "2024-13").unwrap().is_none());
        assert!(search_test(&DataType::Boolean, "yes").unwrap().is_none());
    }
}
