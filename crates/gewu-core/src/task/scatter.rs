//! Bounded scatter samples over an immutable view. Only sampled points cross
//! FFI; native column buffers remain shared. Sampling depends on source IDs,
//! so sorting the view does not change which points appear.

use std::{collections::BinaryHeap, sync::Arc};

use arrow::array::ArrayRef;

use crate::{
    analysis::{numeric::Numeric, AnalysisContext},
    column::format,
    AxisScale, Error, LogicalType, Result,
};

pub const MAX_SCATTER_POINTS: u32 = 4096;

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
    pub version: u64,
    /// None means the zero-based source row index, not display position.
    pub x_column: Option<u32>,
    pub y_column: u32,
    pub total_rows: u64,
    /// Rows with finite, non-null values in both requested axis domains.
    pub eligible_rows: u64,
    /// Finite pairs omitted because a logarithmic axis is zero or negative.
    pub non_positive_rows: u64,
    pub sampled: bool,
    pub points: Vec<ScatterPoint>,
}

/// Independent, terminally cancellable snapshot of a scatter request.
pub struct ScatterAnalysis {
    x: Option<(u32, ArrayRef)>,
    y: (u32, ArrayRef),
    rows: Arc<Vec<u32>>,
    version: u64,
    limit: usize,
    x_scale: AxisScale,
    y_scale: AxisScale,
    context: AnalysisContext,
}

impl ScatterAnalysis {
    pub(crate) fn new(
        x: Option<(u32, ArrayRef)>,
        y: (u32, ArrayRef),
        rows: Arc<Vec<u32>>,
        version: u64,
        limit: u32,
    ) -> Result<Self> {
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
        Ok(Self {
            x,
            y,
            rows,
            version,
            limit: limit as usize,
            x_scale: AxisScale::Linear,
            y_scale: AxisScale::Linear,
            context: AnalysisContext::default(),
        })
    }

    pub fn cancel(&self) {
        self.context.cancel();
    }

    /// Choose independent numeric axis domains. Dates, times and source row
    /// IDs retain a linear scale. Coordinates and labels remain original values.
    pub fn with_scales(mut self, x_scale: AxisScale, y_scale: AxisScale) -> Result<Self> {
        for (scale, array) in [
            (x_scale, self.x.as_ref().map(|(_, array)| array)),
            (y_scale, Some(&self.y.1)),
        ] {
            if scale == AxisScale::Log10
                && !array.is_some_and(|array| {
                    LogicalType::from_arrow(array.data_type()).supports_logarithmic()
                })
            {
                return Err(Error::InvalidArgument(
                    "logarithmic scatter axes require a numeric column".into(),
                ));
            }
        }
        self.x_scale = x_scale;
        self.y_scale = y_scale;
        Ok(self)
    }

    /// Scan the snapshot off the UI thread. Uses O(limit) sample working memory,
    /// excluding shared source buffers and the returned labels' text lengths.
    pub fn run(&self) -> Result<ScatterResult> {
        self.context.check()?;
        let x = self
            .x
            .as_ref()
            .map(|(_, a)| Numeric::new(a).unwrap().accessor());
        let y = Numeric::new(&self.y.1).unwrap().accessor();
        let mut sample = BinaryHeap::with_capacity(self.limit);
        let mut eligible_rows = 0;
        let mut non_positive_rows = 0;
        for (index, &row) in self.rows.iter().enumerate() {
            self.context.check_at(index)?;
            let xv = x
                .as_ref()
                .map_or(Some(f64::from(row)), |get| get(row as usize));
            let (Some(xv), Some(yv)) = (xv, y(row as usize)) else {
                continue;
            };
            if !xv.is_finite() || !yv.is_finite() {
                continue;
            }
            if (self.x_scale == AxisScale::Log10 && xv <= 0.0)
                || (self.y_scale == AxisScale::Log10 && yv <= 0.0)
            {
                non_positive_rows += 1;
                continue;
            }
            eligible_rows += 1;
            let key = (priority(row), row);
            if sample.len() < self.limit {
                sample.push(key);
            } else if sample.peek().is_some_and(|largest| key < *largest) {
                *sample.peek_mut().unwrap() = key;
            }
        }
        let mut ids = sample.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
        ids.sort_unstable();
        let xf = self
            .x
            .as_ref()
            .map(|(_, a)| format::formatter(a.as_ref()))
            .transpose()?;
        let yf = format::formatter(self.y.1.as_ref())?;
        let mut points = Vec::with_capacity(ids.len());
        for row in ids {
            self.context.check()?;
            points.push(ScatterPoint {
                source_row: u64::from(row),
                x: x.as_ref()
                    .map_or(f64::from(row), |get| get(row as usize).unwrap()),
                y: y(row as usize).unwrap(),
                x_label: xf
                    .as_ref()
                    .map_or_else(|| row.to_string(), |f| f.value(row as usize).to_string()),
                y_label: yf.value(row as usize).to_string(),
            });
        }
        self.context.check()?;
        Ok(ScatterResult {
            version: self.version,
            x_column: self.x.as_ref().map(|(column, _)| *column),
            y_column: self.y.0,
            total_rows: self.rows.len() as u64,
            eligible_rows,
            non_positive_rows,
            sampled: eligible_rows > points.len() as u64,
            points,
        })
    }
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
    use arrow::array::{Date32Array, Float64Array, Int64Array, StringArray};

    use super::*;

    #[test]
    fn log_axes_classify_pairs_independently_and_keep_original_values() {
        let x: ArrayRef = Arc::new(Float64Array::from(vec![
            None,
            Some(-3.0),
            Some(0.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(f64::INFINITY),
            Some(5.0),
        ]));
        let y: ArrayRef = Arc::new(Float64Array::from(vec![
            1.0,
            -3.0,
            0.0,
            -2.0,
            2.0,
            f64::INFINITY,
            3.0,
            f64::NAN,
        ]));
        for (xs, ys, eligible, non_positive) in [
            (AxisScale::Linear, AxisScale::Linear, 4, 0),
            (AxisScale::Log10, AxisScale::Linear, 2, 2),
            (AxisScale::Linear, AxisScale::Log10, 1, 3),
            (AxisScale::Log10, AxisScale::Log10, 1, 3),
        ] {
            let result = ScatterAnalysis::new(
                Some((0, x.clone())),
                (1, y.clone()),
                Arc::new((0..8).collect()),
                7,
                8,
            )
            .unwrap()
            .with_scales(xs, ys)
            .unwrap()
            .run()
            .unwrap();
            assert_eq!((result.version, result.total_rows), (7, 8));
            assert_eq!(
                (result.eligible_rows, result.non_positive_rows),
                (eligible, non_positive)
            );
            assert_eq!(result.total_rows - eligible - non_positive, 4);
            let point = result.points.iter().find(|p| p.source_row == 4).unwrap();
            assert_eq!((point.x, point.y), (3.0, 2.0));
            assert_eq!((&*point.x_label, &*point.y_label), ("3.0", "2.0"));
        }

        let dates: ArrayRef = Arc::new(Date32Array::from(vec![1]));
        let job =
            || ScatterAnalysis::new(None, (0, dates.clone()), Arc::new(vec![0]), 0, 8).unwrap();
        assert!(matches!(
            job().with_scales(AxisScale::Log10, AxisScale::Linear),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            job().with_scales(AxisScale::Linear, AxisScale::Log10),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn log_domain_is_applied_before_bounded_sampling() {
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values((0..1000).map(|row| {
            if row >= 990 {
                row
            } else {
                -1
            }
        })));
        let make = |rows| {
            ScatterAnalysis::new(None, (0, array.clone()), Arc::new(rows), 0, 5)
                .unwrap()
                .with_scales(AxisScale::Linear, AxisScale::Log10)
                .unwrap()
        };
        let result = make((0..1000).collect()).run().unwrap();
        assert_eq!((result.eligible_rows, result.non_positive_rows), (10, 990));
        assert!(result.sampled);
        assert_eq!(result.points.len(), 5);
        let ids = |result: ScatterResult| {
            result
                .points
                .into_iter()
                .map(|p| p.source_row)
                .collect::<Vec<_>>()
        };
        assert!(result
            .points
            .iter()
            .all(|p| p.source_row >= 990 && p.y >= 990.0));
        assert_eq!(
            ids(result),
            ids(make((0..1000).rev().collect()).run().unwrap())
        );

        let empty = make((0..990).collect()).run().unwrap();
        assert_eq!((empty.eligible_rows, empty.non_positive_rows), (0, 990));
        assert!(empty.points.is_empty() && !empty.sampled);
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
        let result = ScatterAnalysis::new(
            Some((0, x.clone())),
            (1, y.clone()),
            Arc::new(vec![4, 3, 2, 1, 0]),
            7,
            8,
        )
        .unwrap()
        .run()
        .unwrap();
        assert_eq!(
            (result.total_rows, result.eligible_rows, result.sampled),
            (5, 2, false)
        );
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
        let row_axis = ScatterAnalysis::new(None, (1, y.clone()), Arc::new(vec![4, 0]), 7, 8)
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(row_axis.points[1].x, 4.0);
        assert_eq!(row_axis.points[1].x_label, "4");
        assert_eq!(
            ScatterAnalysis::new(None, (1, y.clone()), Arc::new(vec![]), 7, 8)
                .unwrap()
                .run()
                .unwrap()
                .eligible_rows,
            0
        );
        for limit in [0, MAX_SCATTER_POINTS + 1] {
            assert!(matches!(
                ScatterAnalysis::new(None, (1, y.clone()), Arc::new(vec![0]), 7, limit),
                Err(Error::InvalidArgument(_))
            ));
        }
        let text: ArrayRef = Arc::new(StringArray::from(vec!["bad"]));
        assert!(matches!(
            ScatterAnalysis::new(None, (0, text), Arc::new(vec![0]), 0, 1),
            Err(Error::InvalidArgument(_))
        ));

        let many: ArrayRef = Arc::new(Int64Array::from_iter_values(0..20000));
        let make =
            |rows| ScatterAnalysis::new(None, (0, many.clone()), Arc::new(rows), 0, 64).unwrap();
        let first = make((0..20000).collect()).run().unwrap();
        let reversed = make((0..20000).rev().collect()).run().unwrap();
        assert!(first.sampled);
        assert_eq!(first.eligible_rows, 20000);
        assert_eq!(first.points.len(), 64);
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
        let job = make((0..20000).collect());
        job.cancel();
        assert!(matches!(job.run(), Err(Error::Cancelled)));
    }
}
