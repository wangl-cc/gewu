//! Bounded scatter samples over an immutable view. Only sampled points cross
//! FFI; native column buffers remain shared. Sampling depends on source IDs,
//! so sorting the view does not change which points appear.

use std::{collections::BinaryHeap, sync::Arc};

use arrow::array::ArrayRef;

use crate::{
    analysis::{numeric::Numeric, AnalysisContext},
    format, Error, Result,
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
    /// Rows with finite, non-null values for both coordinates.
    pub eligible_rows: u64,
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
            context: AnalysisContext::default(),
        })
    }

    pub fn cancel(&self) {
        self.context.cancel();
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
        for (index, &row) in self.rows.iter().enumerate() {
            self.context.check_at(index)?;
            let xv = x
                .as_ref()
                .map_or(Some(f64::from(row)), |get| get(row as usize));
            if !xv.is_some_and(f64::is_finite) || !y(row as usize).is_some_and(f64::is_finite) {
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
    use arrow::array::{Float64Array, Int64Array, StringArray};

    use super::*;

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
