//! Temporal text uses the column's unit and time zone, with search prefixes
//! interpreted as periods and explicit bounds as single instants.

use std::sync::Arc;

use arrow::{
    array::{downcast_temporal_array, Array, ArrayRef, AsArray, PrimitiveArray, StringArray},
    compute::cast,
    datatypes::*,
};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use super::decimal::{endpoints, native_range, Endpoint};
use crate::{predicate::Predicate, DataKind, Error, Result};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Result<Option<Predicate<'a>>> {
    let data_type = array.data_type();
    let period = match DataKind::of(data_type) {
        DataKind::Date | DataKind::Timestamp => {
            date_period(text, DataKind::of(data_type) == DataKind::Date)
        }
        DataKind::Time => time_period(text).map(|(start, step)| {
            let start = NaiveDate::default().and_time(start);
            (start, start + step)
        }),
        DataKind::Duration => {
            let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(None);
            }
            let (lo, hi) = endpoints(Some(text), Some(text), 0)?;
            return Ok(Some(bind(array, lo, hi)));
        }
        _ => unreachable!("temporal dispatch"),
    };
    let Some((start, end)) = period else {
        return Ok(None);
    };
    Ok(period_range(data_type, start, end)?.map(|(lo, hi)| {
        bind(
            array,
            Endpoint::Finite(lo.into()),
            Endpoint::Finite(hi.into()),
        )
    }))
}

pub(in crate::column) fn parse_value(data_type: &DataType, text: &str) -> Result<i64> {
    parse_physical(data_type, text)?
        .ok_or_else(|| Error::InvalidArgument(format!("cannot read {text:?} as {data_type}")))
}

/// Cast only the input scalar. Column data always stays in its native array.
fn parse_physical(data_type: &DataType, text: &str) -> Result<Option<i64>> {
    let input: ArrayRef = Arc::new(StringArray::from(vec![text]));
    let typed = cast(&input, data_type)?;
    let typed = typed.as_ref();
    Ok(downcast_temporal_array!(typed => physical_scalar(typed),
        _ => unreachable!("temporal scalar dispatch")))
}

fn physical_scalar<T: ArrowPrimitiveType>(array: &PrimitiveArray<T>) -> Option<i64>
where
    T::Native: Into<i64>,
{
    array.is_valid(0).then(|| array.value(0).into())
}

pub(in crate::column) fn bind(array: &ArrayRef, lo: Endpoint, hi: Endpoint) -> Predicate<'_> {
    downcast_temporal_array!(array => native_range(array, lo, hi),
        DataType::Duration(TimeUnit::Second) => native_range(array.as_primitive::<DurationSecondType>(), lo, hi),
        DataType::Duration(TimeUnit::Millisecond) => native_range(array.as_primitive::<DurationMillisecondType>(), lo, hi),
        DataType::Duration(TimeUnit::Microsecond) => native_range(array.as_primitive::<DurationMicrosecondType>(), lo, hi),
        DataType::Duration(TimeUnit::Nanosecond) => native_range(array.as_primitive::<DurationNanosecondType>(), lo, hi),
        _ => unreachable!("temporal dispatch"))
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
) -> Result<Option<(i64, i64)>> {
    let (Some(lo), Some(hi)) = (physical(data_type, &start)?, physical(data_type, &end)?) else {
        return Ok(None);
    };
    Ok(Some((lo, hi - 1)))
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
    parse_physical(data_type, &text)
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
