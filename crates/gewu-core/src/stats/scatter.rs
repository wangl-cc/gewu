//! Bounded scatter samples over an immutable view. Only sampled points cross
//! FFI; native column buffers remain shared. Sampling depends on source IDs,
//! so sorting the view does not change which points appear.

use std::collections::BinaryHeap;

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{numeric::Numeric, AxisScale};
use crate::{format, Cancellation, Error, Result};

/// Scatter uses the native numeric accessor for approximate coordinates.
pub(crate) fn supports(data_type: &DataType) -> bool {
    super::numeric::supports(data_type)
}

pub const MAX_SCATTER_POINTS: u32 = 4096;

/// Interpretation of scatter labels for axis presentation. Numeric coordinates
/// retain their existing source units; date labels carry the calendar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScatterAxisKind {
    Number,
    Date,
    DateTime,
}

impl ScatterAxisKind {
    fn from_arrow(data_type: &DataType) -> Self {
        match data_type {
            DataType::Date32 | DataType::Date64 => Self::Date,
            DataType::Timestamp(..) => Self::DateTime,
            _ => Self::Number,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScatterPoint {
    pub source_row: u64,
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

/// Scan a borrowed view with a caller-owned cancellation signal.
pub(crate) fn analyze(
    x: Option<(u32, &ArrayRef)>,
    y: (u32, &ArrayRef),
    rows: &[u32],
    limit: u32,
    x_scale: AxisScale,
    y_scale: AxisScale,
    context: &Cancellation,
) -> Result<ScatterResult> {
    context.check()?;
    if !(1..=MAX_SCATTER_POINTS).contains(&limit) {
        return Err(Error::InvalidArgument(format!(
            "scatter limit must be between 1 and {MAX_SCATTER_POINTS}"
        )));
    }
    for (_, array) in x.iter().chain(std::iter::once(&y)) {
        if Numeric::new(array).is_none() {
            return Err(Error::InvalidArgument(format!(
                "{} does not support scatter coordinates",
                array.data_type()
            )));
        }
    }
    for (scale, array) in [(x_scale, x.as_ref().map(|(_, a)| *a)), (y_scale, Some(y.1))] {
        if scale == AxisScale::Log10
            && !array.is_some_and(|array| super::supports_logarithmic(array.data_type()))
        {
            return Err(Error::InvalidArgument(
                "logarithmic scatter axes require a numeric column".into(),
            ));
        }
    }
    let limit = limit as usize;

    context.check()?;
    let read_x = x.as_ref().map(|(_, a)| Numeric::new(a).unwrap().accessor());
    let read_y = Numeric::new(y.1).unwrap().accessor();
    let mut sample = BinaryHeap::with_capacity(limit);
    let mut eligible_rows = 0;
    let mut non_positive_rows = 0;
    for (index, &row) in rows.iter().enumerate() {
        context.check_at(index)?;
        let xv = read_x
            .as_ref()
            .map_or(Some(f64::from(row)), |get| get(row as usize));
        let (Some(xv), Some(yv)) = (xv, read_y(row as usize)) else {
            continue;
        };
        if !xv.is_finite() || !yv.is_finite() {
            continue;
        }
        if (x_scale == AxisScale::Log10 && xv <= 0.0) || (y_scale == AxisScale::Log10 && yv <= 0.0)
        {
            non_positive_rows += 1;
            continue;
        }
        eligible_rows += 1;
        let key = (priority(row), row);
        if sample.len() < limit {
            sample.push(key);
        } else if sample.peek().is_some_and(|largest| key < *largest) {
            *sample.peek_mut().unwrap() = key;
        }
    }
    let mut ids = sample.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    ids.sort_unstable();
    let xf = x
        .as_ref()
        .map(|(_, a)| format::formatter(a.as_ref()))
        .transpose()?;
    let yf = format::formatter(y.1.as_ref())?;
    let mut points = Vec::with_capacity(ids.len());
    for row in ids {
        context.check()?;
        points.push(ScatterPoint {
            source_row: u64::from(row),
            x: read_x
                .as_ref()
                .map_or(f64::from(row), |get| get(row as usize).unwrap()),
            y: read_y(row as usize).unwrap(),
            x_label: xf
                .as_ref()
                .map_or_else(|| row.to_string(), |f| f.value(row as usize).to_string()),
            y_label: yf.value(row as usize).to_string(),
        });
    }
    context.check()?;
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
        let c = Cancellation::default();
        let result = analyze(
            Some((0, &dates)),
            (1, &timestamps),
            &[],
            8,
            AxisScale::Linear,
            AxisScale::Linear,
            &c,
        )
        .unwrap();
        assert!(result.points.is_empty());
        assert_eq!(result.x_axis_kind, ScatterAxisKind::Date);
        assert_eq!(result.y_axis_kind, ScatterAxisKind::DateTime);
        let result = analyze(
            None,
            (0, &dates),
            &[0],
            8,
            AxisScale::Linear,
            AxisScale::Linear,
            &c,
        )
        .unwrap();
        assert_eq!(result.x_axis_kind, ScatterAxisKind::Number);
        assert_eq!((result.points[0].x, result.points[0].y), (0.0, 1.0));
        for (x, y) in [
            (AxisScale::Log10, AxisScale::Linear),
            (AxisScale::Linear, AxisScale::Log10),
        ] {
            assert!(matches!(
                analyze(None, (0, &dates), &[0], 8, x, y, &c),
                Err(Error::InvalidArgument(_))
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
        for (xs, ys, eligible, excluded) in [
            (AxisScale::Linear, AxisScale::Linear, 4, 0),
            (AxisScale::Log10, AxisScale::Linear, 2, 2),
            (AxisScale::Linear, AxisScale::Log10, 1, 3),
            (AxisScale::Log10, AxisScale::Log10, 1, 3),
        ] {
            let result = analyze(
                Some((0, &x)),
                (1, &y),
                &(0..8).collect::<Vec<_>>(),
                8,
                xs,
                ys,
                &Cancellation::default(),
            )
            .unwrap();
            assert_eq!(
                (result.eligible_rows, result.non_positive_rows),
                (eligible, excluded)
            );
            let point = result.points.iter().find(|p| p.source_row == 4).unwrap();
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
        let make = |rows: Vec<u32>| {
            analyze(
                None,
                (0, &array),
                &rows,
                5,
                AxisScale::Linear,
                AxisScale::Log10,
                &Cancellation::default(),
            )
            .unwrap()
        };
        let result = make((0..1000).collect());
        assert_eq!((result.eligible_rows, result.non_positive_rows), (10, 990));
        assert!(result.eligible_rows > result.points.len() as u64);
        assert_eq!(result.points.len(), 5);
        assert!(result
            .points
            .iter()
            .all(|p| p.source_row >= 990 && p.y >= 990.0));
        let ids = |r: ScatterResult| {
            r.points
                .into_iter()
                .map(|p| p.source_row)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(result), ids(make((0..1000).rev().collect())));
        let empty = make((0..990).collect());
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
        let c = Cancellation::default();
        let result = analyze(
            Some((0, &x)),
            (1, &y),
            &[4, 3, 2, 1, 0],
            8,
            AxisScale::Linear,
            AxisScale::Linear,
            &c,
        )
        .unwrap();
        assert_eq!((result.eligible_rows, result.points.len()), (2, 2));
        assert_eq!(
            result
                .points
                .iter()
                .map(|p| p.source_row)
                .collect::<Vec<_>>(),
            [0, 4]
        );
        assert_eq!(result.points[0].x_label, "9007199254740993");
        assert_eq!((result.points[0].y, result.points[1].y), (1.5, 5.5));
        let row_axis = analyze(
            None,
            (1, &y),
            &[4, 0],
            8,
            AxisScale::Linear,
            AxisScale::Linear,
            &c,
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
                    (1, &y),
                    &[0],
                    limit,
                    AxisScale::Linear,
                    AxisScale::Linear,
                    &c
                ),
                Err(Error::InvalidArgument(_))
            ));
        }
        let text: ArrayRef = Arc::new(StringArray::from(vec!["bad"]));
        assert!(matches!(
            analyze(
                None,
                (0, &text),
                &[0],
                1,
                AxisScale::Linear,
                AxisScale::Linear,
                &c
            ),
            Err(Error::InvalidArgument(_))
        ));
        let many: ArrayRef = Arc::new(Int64Array::from_iter_values(0..20000));
        let sample = |rows: Vec<u32>, cancel: &Cancellation| {
            analyze(
                None,
                (0, &many),
                &rows,
                64,
                AxisScale::Linear,
                AxisScale::Linear,
                cancel,
            )
        };
        let first = sample((0..20000).collect(), &c).unwrap();
        let reversed = sample((0..20000).rev().collect(), &c).unwrap();
        assert!(first.eligible_rows > first.points.len() as u64);
        assert_eq!((first.eligible_rows, first.points.len()), (20000, 64));
        assert_eq!(
            first
                .points
                .iter()
                .map(|p| p.source_row)
                .collect::<Vec<_>>(),
            reversed
                .points
                .iter()
                .map(|p| p.source_row)
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            sample((0..20000).collect(), &Cancellation::after_checks(5)),
            Err(Error::Cancelled)
        ));
    }
}
