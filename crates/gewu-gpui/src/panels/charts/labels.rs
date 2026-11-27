//! Short axis labels. Keep original strings for hover, copying and filter bounds.
use chrono::{DateTime, Utc};

pub(super) struct Endpoints {
    pub lower: String,
    pub upper: String,
    pub context: Option<String>,
}

struct Timestamp<'a> {
    date: &'a str,
    time: &'a str,
    zone: &'a str,
}

fn timestamp(text: &str) -> Option<Timestamp<'_>> {
    let date = text.get(..10)?;
    if !date.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    }) {
        return None;
    }
    let rest = text.get(10..)?;
    let time = if rest.is_empty() {
        ""
    } else {
        rest.strip_prefix(['T', ' '])?
    };
    let (time, zone) = if let Some(t) = time.strip_suffix('Z') {
        (t, "Z")
    } else if let Some(suffix) = time.len().checked_sub(6).and_then(|i| time.get(i..))
        && suffix.starts_with(['+', '-'])
        && suffix[1..].bytes().enumerate().all(|(i, b)| {
            if i == 2 {
                b == b':'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        time.split_at(time.len() - 6)
    } else {
        (time, "")
    };
    Some(Timestamp { date, time, zone })
}

pub(super) fn compact(text: &str) -> String {
    if text.len() <= 12 {
        return text.to_owned();
    }
    scientific(text).unwrap_or_else(|| text.to_owned())
}

fn scientific(text: &str) -> Option<String> {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exp) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some("0".into());
    }
    let offset = (whole.len() as i64) - (digits.len() - significant.len()) as i64 - 1;
    let exponent = exp.checked_add(offset)?;
    let head = &significant[..1];
    let tail = &significant[1..4.min(significant.len())];
    let sign = if text.starts_with('-') { "-" } else { "" };
    let dot = if tail.is_empty() { "" } else { "." };
    let more = if significant.len() > 4 { "…" } else { "" };
    Some(format!("{sign}{head}{dot}{tail}{more}e{exponent}"))
}

fn times<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    for n in [5, 8] {
        if let (Some(left), Some(right)) = (a.get(..n), b.get(..n))
            && left != right
        {
            return (left, right);
        }
    }
    (a, b)
}

pub(super) fn endpoints(lower: &str, upper: &str) -> Endpoints {
    if let (Some(a), Some(b)) = (timestamp(lower), timestamp(upper)) {
        let (mut left, mut right) = if a.date != b.date {
            if a.date[..4] != b.date[..4] {
                (&a.date[..7], &b.date[..7])
            } else {
                (&a.date[5..], &b.date[5..])
            }
        } else if !a.time.is_empty() && !b.time.is_empty() {
            times(a.time, b.time)
        } else {
            (a.date, b.date)
        };
        let mut context = Vec::new();
        if a.date == b.date && !a.time.is_empty() {
            context.push(a.date)
        } else if a.date[..4] == b.date[..4] {
            context.push(&a.date[..4])
        }
        if a.zone == b.zone && !a.zone.is_empty() {
            context.push(if a.zone == "Z" { "UTC" } else { a.zone })
        }
        let zones = if a.zone != b.zone {
            Some((format!("{left} {}", a.zone), format!("{right} {}", b.zone)))
        } else {
            None
        };
        if let Some((x, y)) = &zones {
            left = x;
            right = y;
        }
        Endpoints {
            lower: left.into(),
            upper: right.into(),
            context: (!context.is_empty()).then(|| context.join(" · ")),
        }
    } else if lower.contains(':')
        && upper.contains(':')
        && !lower.contains(' ')
        && !upper.contains(' ')
    {
        let (a, b) = times(lower, upper);
        Endpoints {
            lower: a.into(),
            upper: b.into(),
            context: None,
        }
    } else {
        Endpoints {
            lower: compact(lower),
            upper: compact(upper),
            context: None,
        }
    }
}

pub(super) fn utc(seconds: f64) -> Option<DateTime<Utc>> {
    if !seconds.is_finite() {
        return None;
    }
    let secs = seconds.floor() as i64;
    let nanos = ((seconds - secs as f64) * 1e9)
        .round()
        .clamp(0., 999_999_999.) as u32;
    DateTime::from_timestamp(secs, nanos)
}

pub(crate) use crate::ui::messages::significant_digits;

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
