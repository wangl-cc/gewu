//! Bounded scatter samples over an immutable view. Only sampled points cross
//! FFI; native column buffers remain shared. Sampling depends on source IDs,
//! so sorting the view does not change which points appear.

use std::collections::BinaryHeap;

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{numeric::Numeric, AxisScale};
use crate::{col_view::Selection, format, row_ids::RowSlice, stats::Result, ColumnId};

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
    x: Option<(ColumnId, &ArrayRef)>,
    y: (ColumnId, &ArrayRef),
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
    for (_, array) in x.iter().chain(std::iter::once(&y)) {
        if Numeric::new(array).is_none() {
            return Err(crate::AnalysisError::UnsupportedOperation {
                operation: crate::AnalysisOperation::Scatter,
                data_type: array.data_type().clone(),
            });
        }
    }
    for (scale, array) in [(x_scale, x.as_ref().map(|(_, a)| *a)), (y_scale, Some(y.1))] {
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
    let read_x = x.as_ref().map(|(_, a)| Numeric::new(a).unwrap());
    let read_y = Numeric::new(y.1).unwrap();
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
        .as_ref()
        .map(|(_, a)| format::formatter(a.as_ref()))
        .transpose()?;
    let yf = format::formatter(y.1.as_ref())?;
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
        x_axis_kind: x.as_ref().map_or(ScatterAxisKind::Number, |(_, array)| {
            ScatterAxisKind::from_arrow(array.data_type())
        }),
        y_axis_kind: ScatterAxisKind::from_arrow(y.1.data_type()),
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
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        Date32Array, Float64Array, Int64Array, StringArray, TimestampNanosecondArray,
    };

    use super::*;

    #[test]
    fn axis_kinds_describe_the_snapshot_even_without_points() {
        let dates: ArrayRef = Arc::new(Date32Array::from(vec![1]));
        let timestamps: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![0]));
        let c0 = ColumnId::new(0);
        let c1 = ColumnId::new(1);
        let result = analyze(
            Some((c0, &dates)),
            (c1, &timestamps),
            RowSlice::Selected(&[]),
            8,
            AxisScale::Linear,
            AxisScale::Linear,
        )
        .unwrap();
        assert!(result.points.is_empty());
        assert_eq!(result.x_axis_kind, ScatterAxisKind::Date {
            unit: TimeUnit::Day
        });
        assert_eq!(result.y_axis_kind, ScatterAxisKind::DateTime {
            unit: TimeUnit::Nanosecond
        });
        let result = analyze(
            None,
            (c0, &dates),
            RowSlice::All(1),
            8,
            AxisScale::Linear,
            AxisScale::Linear,
        )
        .unwrap();
        assert_eq!(result.x_axis_kind, ScatterAxisKind::Number);
        assert_eq!((result.points[0].x, result.points[0].y), (0.0, 1.0));
        for (x, y) in [
            (AxisScale::Log10, AxisScale::Linear),
            (AxisScale::Linear, AxisScale::Log10),
        ] {
            assert!(matches!(
                analyze(None, (c0, &dates), RowSlice::All(1), 8, x, y),
                Err(crate::AnalysisError::UnsupportedOperation { .. }
                    | crate::AnalysisError::UnsupportedAxisScale { .. })
            ));
        }
    }

    #[test]
    fn log_axes_classify_pairs_independently_and_keep_original_values() {
        let x: ArrayRef = Arc::new(Float64Array::from(vec![
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(0.0),
            Some(3.0),
            Some(-1.0),
            Some(2.0),
            Some(4.0),
        ]));
        let y: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(1.0),
            Some(1.0),
            Some(1.0),
            Some(-1.0),
            Some(2.0),
            Some(-2.0),
            Some(0.0),
            None,
        ]));
        let c0 = ColumnId::new(0);
        let c1 = ColumnId::new(1);
        for (xs, ys, eligible, excluded) in [
            (AxisScale::Linear, AxisScale::Linear, 4, 0),
            (AxisScale::Log10, AxisScale::Linear, 2, 2),
            (AxisScale::Linear, AxisScale::Log10, 1, 3),
            (AxisScale::Log10, AxisScale::Log10, 1, 3),
        ] {
            let result = analyze(Some((c0, &x)), (c1, &y), RowSlice::All(8), 8, xs, ys).unwrap();
            assert_eq!(
                (result.eligible_rows, result.non_positive_rows),
                (eligible, excluded)
            );
            let point = result
                .points
                .iter()
                .find(|p| p.source_row == crate::SourceRow(4))
                .unwrap();
            assert_eq!((point.x, point.y), (3.0, 2.0));
            assert_eq!((&*point.x_label, &*point.y_label), ("3.0", "2.0"));
        }
    }

    #[test]
    fn log_domain_is_applied_before_bounded_sampling() {
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values((0..1000).map(|r| {
            if r >= 990 {
                r
            } else {
                -1
            }
        })));
        let c0 = ColumnId::new(0);
        let make = |rows: RowSlice<'_>| {
            analyze(
                None,
                (c0, &array),
                rows,
                5,
                AxisScale::Linear,
                AxisScale::Log10,
            )
            .unwrap()
        };
        let result = make(RowSlice::All(1000));
        assert_eq!((result.eligible_rows, result.non_positive_rows), (10, 990));
        assert!(result.eligible_rows > result.points.len() as u64);
        assert_eq!(result.points.len(), 5);
        assert!(result
            .points
            .iter()
            .all(|p| p.source_row.0 >= 990 && p.y >= 990.0));
        let ids = |r: ScatterResult| {
            r.points
                .into_iter()
                .map(|p| p.source_row.0)
                .collect::<Vec<_>>()
        };
        let rev: Vec<u32> = (0..1000).rev().collect();
        assert_eq!(ids(result), ids(make(RowSlice::Selected(&rev))));
        let empty = make(RowSlice::All(990));
        assert_eq!((empty.eligible_rows, empty.non_positive_rows), (0, 990));
        assert!(empty.points.is_empty() && empty.eligible_rows == 0);
    }

    #[test]
    fn scatter_preserves_pairs_filters_invalid_values_and_limits_memory() {
        let x: ArrayRef = Arc::new(Int64Array::from(vec![
            Some(9007199254740993),
            Some(2),
            Some(3),
            None,
            Some(5),
        ]));
        let y: ArrayRef = Arc::new(Float64Array::from(vec![
            1.5,
            f64::NAN,
            f64::INFINITY,
            4.0,
            5.5,
        ]));
        let c0 = ColumnId::new(0);
        let c1 = ColumnId::new(1);
        let result = analyze(
            Some((c0, &x)),
            (c1, &y),
            RowSlice::Selected(&[4, 3, 2, 1, 0]),
            8,
            AxisScale::Linear,
            AxisScale::Linear,
        )
        .unwrap();
        assert_eq!((result.eligible_rows, result.points.len()), (2, 2));
        assert_eq!(
            result
                .points
                .iter()
                .map(|p| p.source_row.0)
                .collect::<Vec<_>>(),
            [0, 4]
        );
        assert_eq!(result.points[0].x_label, "9007199254740993");
        assert_eq!((result.points[0].y, result.points[1].y), (1.5, 5.5));
        let row_axis = analyze(
            None,
            (c1, &y),
            RowSlice::Selected(&[4, 0]),
            8,
            AxisScale::Linear,
            AxisScale::Linear,
        )
        .unwrap();
        assert_eq!(
            (row_axis.points[1].x, row_axis.points[1].x_label.as_str()),
            (4.0, "4")
        );
        for limit in [0, MAX_SCATTER_POINTS + 1] {
            assert!(matches!(
                analyze(
                    None,
                    (c1, &y),
                    RowSlice::Selected(&[0]),
                    limit,
                    AxisScale::Linear,
                    AxisScale::Linear,
                ),
                Err(crate::AnalysisError::InvalidScatterLimit { .. })
            ));
        }
        let text: ArrayRef = Arc::new(StringArray::from(vec!["bad"]));
        assert!(matches!(
            analyze(
                None,
                (c0, &text),
                RowSlice::All(1),
                1,
                AxisScale::Linear,
                AxisScale::Linear,
            ),
            Err(crate::AnalysisError::UnsupportedOperation { .. })
        ));
        let many: ArrayRef = Arc::new(Int64Array::from_iter_values(0..20000));
        let sample = |rows: RowSlice<'_>| {
            analyze(
                None,
                (c0, &many),
                rows,
                64,
                AxisScale::Linear,
                AxisScale::Linear,
            )
        };
        let first = sample(RowSlice::All(20000)).unwrap();
        let rev: Vec<u32> = (0..20000).rev().collect();
        let reversed = sample(RowSlice::Selected(&rev)).unwrap();
        assert!(first.eligible_rows > first.points.len() as u64);
        assert_eq!((first.eligible_rows, first.points.len()), (20000, 64));
        assert_eq!(
            first
                .points
                .iter()
                .map(|p| p.source_row.0)
                .collect::<Vec<_>>(),
            reversed
                .points
                .iter()
                .map(|p| p.source_row.0)
                .collect::<Vec<_>>()
        );
    }
}
