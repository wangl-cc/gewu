use chrono::Datelike;
use gewu_core::{AxisScale, ScatterAxisKind};

use super::labels;

#[derive(Clone)]
pub(super) enum Label {
    Text(String),
    Row(u64),
}
impl Label {
    pub(super) fn text(&self, base: u64) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Row(row) => row.saturating_add(base).to_string(),
        }
    }
}
pub(super) struct Tick {
    pub fraction: f32,
    pub label: Label,
}
pub(super) struct Axis {
    lo: f64,
    hi: f64,
    scale: AxisScale,
    pub ticks: Vec<Tick>,
    pub context: Option<String>,
}

pub(super) fn seconds(value: f64, kind: ScatterAxisKind) -> f64 {
    match kind {
        ScatterAxisKind::Number => value,
        ScatterAxisKind::Date { unit } | ScatterAxisKind::DateTime { unit } => {
            // Dividing by an integer power of ten avoids rounding the unit conversion twice.
            match unit {
                gewu_core::TimeUnit::Day => value * 86400.,
                gewu_core::TimeUnit::Second => value,
                gewu_core::TimeUnit::Millisecond => value / 1000.,
                gewu_core::TimeUnit::Microsecond => value / 1_000_000.,
                gewu_core::TimeUnit::Nanosecond => value / 1_000_000_000.,
            }
        }
    }
}

fn logarithmic_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    if count == 0 || !lo.is_finite() || !hi.is_finite() || lo <= 0. || lo > hi {
        return Vec::new();
    }
    if lo == hi || count < 2 {
        return vec![lo];
    }
    let first = lo.log10().ceil();
    let last = hi.log10().floor();
    if first >= last {
        return vec![lo, hi];
    }
    let n = count.min((last - first) as usize + 1);
    let ticks = (0..n)
        .map(|i| 10f64.powf((first + (last - first) * i as f64 / (n - 1) as f64).round()))
        .filter(|v| v.is_finite() && *v >= lo && *v <= hi)
        .collect::<Vec<_>>();
    if ticks.is_empty() {
        vec![lo, hi]
    } else {
        ticks
    }
}

/// Choose readable decimal intervals within the data domain. Use exact endpoints when
/// the interval cannot be represented, for example across both extremes of f64.
fn numeric_ticks(lo: f64, hi: f64, count: usize, rows: bool) -> Vec<f64> {
    if lo == hi || count < 2 {
        return vec![lo];
    }
    let spacing = (hi - lo) / (count - 1) as f64;
    let power = 10f64.powf(spacing.log10().floor());
    let multiple = match spacing / power {
        m if m < 1.5 => 1.,
        m if m < 3.5 => 2.,
        m if m < 7.5 => 5.,
        _ => 10.,
    };
    let mut step = power * multiple;
    if !step.is_finite() || step <= 0. {
        return vec![lo, hi];
    }
    if rows {
        step = step.max(1.);
    }
    let first = (lo / step).ceil();
    let mut ticks: Vec<_> = (0..count * 2)
        .map(|i| (first + i as f64) * step)
        .filter(|v| v.is_finite() && (lo..=hi).contains(v))
        .collect();
    ticks.dedup();
    if ticks.len() < 2 { vec![lo, hi] } else { ticks }
}

impl Axis {
    pub fn for_points(
        values: &[f64],
        labels: impl Fn(usize) -> String,
        kind: ScatterAxisKind,
        scale: AxisScale,
        rows: bool,
        count: usize,
    ) -> Option<Self> {
        let lower = (0..values.len()).min_by(|a, b| values[*a].total_cmp(&values[*b]))?;
        let upper = (0..values.len()).max_by(|a, b| values[*a].total_cmp(&values[*b]))?;
        let (lo, hi) = (values[lower], values[upper]);
        let mut axis = Self {
            lo,
            hi,
            scale,
            ticks: Vec::new(),
            context: None,
        };
        let is_temporal = kind != ScatterAxisKind::Number;
        let mut ticks = if scale == AxisScale::Log10 {
            logarithmic_ticks(lo, hi, count)
        } else if !is_temporal {
            numeric_ticks(lo, hi, count, rows)
        } else if hi == lo {
            vec![lo]
        } else {
            (0..count)
                .map(|i| {
                    let part = i as f64 / (count - 1) as f64;
                    if i == 0 {
                        lo
                    } else if i == count - 1 {
                        hi
                    } else {
                        lo * (1. - part) + hi * part
                    }
                })
                .collect()
        };
        ticks.dedup();
        let endpoints = labels::endpoints(&labels(lower), &labels(upper));
        let utc_range = if is_temporal {
            labels::utc(lo).zip(labels::utc(hi))
        } else {
            None
        };
        // Datetimes are plotted in seconds and labelled in UTC; hover always uses Core's exact
        // original text, including its zone and any fractional precision beyond f64.
        if let Some((a, b)) = utc_range {
            axis.context = Some(if a.date_naive() == b.date_naive() {
                format!("{} · UTC", a.date_naive())
            } else if a.year() == b.year() {
                format!("{} · UTC", a.year())
            } else {
                format!("{}–{} · UTC", a.year(), b.year())
            });
        }
        for v in ticks {
            let label = if rows {
                Label::Row(v as u64)
            } else if scale == AxisScale::Log10 {
                Label::Text(format!("{v:e}"))
            } else if let Some((a, b)) = utc_range
                && let Some(t) = labels::utc(v)
            {
                let pattern = if a.date_naive() == b.date_naive() {
                    if hi - lo >= 60. {
                        "%H:%M"
                    } else if hi - lo >= 1. {
                        "%H:%M:%S"
                    } else {
                        "%H:%M:%S%.f"
                    }
                } else if a.year() == b.year() {
                    "%m-%d"
                } else {
                    "%Y-%m"
                };
                Label::Text(t.format(pattern).to_string())
            } else if v == lo {
                Label::Text(endpoints.lower.clone())
            } else if v == hi {
                Label::Text(endpoints.upper.clone())
            } else {
                Label::Text(labels::significant_digits(v, 3))
            };
            axis.ticks.push(Tick {
                fraction: axis.fraction(v),
                label,
            });
        }
        Some(axis)
    }

    pub fn fraction(&self, v: f64) -> f32 {
        let transform = |n: f64| {
            if self.scale == AxisScale::Log10 {
                n.log10()
            } else {
                n
            }
        };
        let (lo, hi, v) = (transform(self.lo), transform(self.hi), transform(v));
        if lo == hi {
            0.5
        } else if (hi - lo).is_finite() {
            ((v - lo) / (hi - lo)) as f32
        } else {
            ((v * 0.5 - lo * 0.5) / (hi * 0.5 - lo * 0.5)) as f32
        }
    }
}

#[cfg(test)]
#[path = "axis_tests.rs"]
mod tests;
