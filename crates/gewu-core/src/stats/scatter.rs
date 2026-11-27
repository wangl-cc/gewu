//! Bounded scatter samples over an immutable view. Only sampled points cross
//! FFI; native column buffers remain shared. Sampling depends on source IDs,
//! so sorting the view does not change which points appear.

use std::collections::BinaryHeap;

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{numeric::Numeric, AxisScale};
use crate::{col_view::Selection, format, row_ids::RowSlice, stats::Result};

/// Scatter uses the native numeric accessor for approximate coordinates.
pub(crate) fn supports(data_type: &DataType) -> bool {
    super::numeric::supports(data_type)
}

pub const MAX_SCATTER_POINTS: u32 = 4096;

/// Interpretation of scatter coordinates for axis presentation. Coordinates
/// keep the column's native units; temporal axes report that unit so a
/// coordinate times [`TimeUnit::seconds`] is seconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScatterAxisKind {
    Number,
    Date { unit: TimeUnit },
    DateTime { unit: TimeUnit },
}

/// The native unit of a temporal coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Day,
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl TimeUnit {
    /// Seconds in one unit.
    pub fn seconds(self) -> f64 {
        match self {
            Self::Day => 86_400.0,
            Self::Second => 1.0,
            Self::Millisecond => 1e-3,
            Self::Microsecond => 1e-6,
            Self::Nanosecond => 1e-9,
        }
    }
}

impl From<arrow::datatypes::TimeUnit> for TimeUnit {
    fn from(unit: arrow::datatypes::TimeUnit) -> Self {
        match unit {
            arrow::datatypes::TimeUnit::Second => Self::Second,
            arrow::datatypes::TimeUnit::Millisecond => Self::Millisecond,
            arrow::datatypes::TimeUnit::Microsecond => Self::Microsecond,
            arrow::datatypes::TimeUnit::Nanosecond => Self::Nanosecond,
        }
    }
}

impl ScatterAxisKind {
    fn from_arrow(data_type: &DataType) -> Self {
        match data_type {
            DataType::Date32 => Self::Date {
                unit: TimeUnit::Day,
            },
            DataType::Date64 => Self::Date {
                unit: TimeUnit::Millisecond,
            },
            DataType::Timestamp(unit, _) => Self::DateTime {
                unit: (*unit).into(),
            },
            _ => Self::Number,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScatterPoint {
    pub source_row: crate::SourceRow,
    /// Approximate plot coordinates; use labels for original precision.
    pub x: f64,
    pub y: f64,
    pub x_label: String,
    pub y_label: String,
}

#[derive(Debug, Clone)]
pub struct ScatterResult {
    pub x_axis_kind: ScatterAxisKind,
    pub y_axis_kind: ScatterAxisKind,
    /// Rows with finite, non-null values in both requested axis domains.
    pub eligible_rows: u64,
    /// Finite pairs omitted because a logarithmic axis is zero or negative.
    pub non_positive_rows: u64,
    pub points: Vec<ScatterPoint>,
}

/// Rows converted to `f64` per scan step, bounding scratch memory.
const SCAN_CHUNK: usize = 4096;

/// Scan a borrowed view for a bounded scatter sample.
pub(crate) fn analyze(
    x: Option<&ArrayRef>,
    y: &ArrayRef,
    rows: RowSlice<'_>,
    limit: u32,
    x_scale: AxisScale,
    y_scale: AxisScale,
) -> Result<ScatterResult> {
    if !(1..=MAX_SCATTER_POINTS).contains(&limit) {
        return Err(crate::AnalysisError::InvalidScatterLimit {
            value: limit,
            min: 1,
            max: MAX_SCATTER_POINTS,
        });
    }
    let read_x = x
        .map(|array| Numeric::for_operation(array, crate::AnalysisOperation::Scatter))
        .transpose()?;
    let read_y = Numeric::for_operation(y, crate::AnalysisOperation::Scatter)?;
    for (scale, array) in [(x_scale, x), (y_scale, Some(y))] {
        if scale == AxisScale::Log10
            && !array.is_some_and(|array| super::supports_logarithmic(array.data_type()))
        {
            return Err(match array {
                Some(array) => crate::AnalysisError::UnsupportedOperation {
                    operation: crate::AnalysisOperation::Scatter,
                    data_type: array.data_type().clone(),
                },
                None => crate::AnalysisError::UnsupportedAxisScale {
                    operation: crate::AnalysisOperation::Scatter,
                    scale,
                },
            });
        }
    }
    let limit = limit as usize;

    // Columnar scan: convert fixed-size chunks of the selection to f64 (nulls
    // become NaN and are rejected with other non-finite values), then sample
    // by source row.
    let len = rows.len();
    let chunk = SCAN_CHUNK.min(len);
    let (mut xs, mut ys) = (vec![0.0; chunk], vec![0.0; chunk]);
    let mut sample = BinaryHeap::with_capacity(limit);
    let mut eligible_rows = 0;
    let mut non_positive_rows = 0;
    for start in (0..len).step_by(SCAN_CHUNK) {
        let window = rows.window(start, (start + SCAN_CHUNK).min(len));
        let n = window.len();
        let selection = Selection::from(window);
        let (xs, ys) = (&mut xs[..n], &mut ys[..n]);
        read_y.write_f64(&selection, ys);
        match read_x {
            Some(read_x) => read_x.write_f64(&selection, xs),
            None => {
                for (x, row) in xs.iter_mut().zip(window.iter()) {
                    *x = row as f64;
                }
            }
        }
        for ((&xv, &yv), row) in xs.iter().zip(ys.iter()).zip(window.iter()) {
            if !xv.is_finite() || !yv.is_finite() {
                continue;
            }
            if (x_scale == AxisScale::Log10 && xv <= 0.0)
                || (y_scale == AxisScale::Log10 && yv <= 0.0)
            {
                non_positive_rows += 1;
                continue;
            }
            eligible_rows += 1;
            let row = row as u32;
            let key = (priority(row), row);
            if sample.len() < limit {
                sample.push(key);
            } else if sample.peek().is_some_and(|largest| key < *largest) {
                *sample.peek_mut().unwrap() = key;
            }
        }
    }
    let mut ids = sample.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    ids.sort_unstable();
    let xf = x
        .map(|array| format::formatter(array.as_ref()))
        .transpose()?;
    let yf = format::formatter(y.as_ref())?;
    let read = |numeric: Numeric<'_>, row: usize| {
        let mut value = 0.0;
        numeric.write_f64(
            &Selection::Range(row..row + 1),
            std::slice::from_mut(&mut value),
        );
        value
    };
    let mut points = Vec::with_capacity(ids.len());
    for row in ids {
        let index = row as usize;
        points.push(ScatterPoint {
            source_row: crate::SourceRow(u64::from(row)),
            x: read_x.map_or(f64::from(row), |numeric| read(numeric, index)),
            y: read(read_y, index),
            x_label: xf
                .as_ref()
                .map_or_else(|| row.to_string(), |f| f.value(index).to_string()),
            y_label: yf.value(index).to_string(),
        });
    }
    Ok(ScatterResult {
        x_axis_kind: x.map_or(ScatterAxisKind::Number, |array| {
            ScatterAxisKind::from_arrow(array.data_type())
        }),
        y_axis_kind: ScatterAxisKind::from_arrow(y.data_type()),
        eligible_rows,
        non_positive_rows,
        points,
    })
}

fn priority(row: u32) -> u64 {
    // SplitMix64's fixed permutation gives reproducible, well-spread priorities
    // without allocating a random generator or depending on row traversal order.
    let mut value = u64::from(row).wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

#[cfg(test)]
#[path = "scatter_tests.rs"]
mod tests;
