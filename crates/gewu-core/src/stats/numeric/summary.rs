//! Native extrema and moments, with scalar conversion for mean and variance.

use arrow::{array::PrimitiveArray, datatypes::ArrowPrimitiveType};

use super::{IntegerColumn, RealColumn, RealType};
use crate::col_view::Selection;

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128>,
{
    pub(super) fn summarize(self, selection: &Selection<'_>) -> NumericSummary {
        summarize(self.0, selection, |v| v.into() as f64)
    }
}

impl<'a, T: RealType> RealColumn<'a, T> {
    pub(super) fn summarize(self, selection: &Selection<'_>) -> NumericSummary {
        summarize(self.array, selection, |v| T::as_float(v, self.scale))
    }
}

/// Numeric results before formatting extrema and applying display
/// policies.
#[derive(Default)]
pub(super) struct NumericSummary {
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub nan_count: u64,
    pub min_row: Option<usize>,
    pub max_row: Option<usize>,
    pub infinite_count: u64,
}

fn summarize<T, F>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    as_float: F,
) -> NumericSummary
where
    T: ArrowPrimitiveType,
    F: Fn(T::Native) -> f64,
{
    let mut summary = NumericSummary::default();
    let mut infinite_count = 0;
    let (mut n, mut mean, mut m2) = (0u64, 0.0, 0.0);
    let mut scale = f64::MIN_POSITIVE;
    // (value, row) of the smallest and largest so far.
    let mut min: Option<(T::Native, usize)> = None;
    let mut max: Option<(T::Native, usize)> = None;
    crate::stats::scan::for_each_valid(array, selection, |row, native| {
        let value = as_float(native);
        if value.is_nan() {
            summary.nan_count += 1;
            return;
        }
        infinite_count += u64::from(value.is_infinite());
        // Welford in power-of-two units keeps differences and squared differences
        // bounded, without rounding away low bits of large, nearby values. Use
        // the smallest normal scale for subnormals; never normalize by infinity.
        if value.is_finite() {
            let next_scale = f64::from_bits(value.to_bits() & 0x7ff0_0000_0000_0000);
            if next_scale > scale {
                let ratio = scale / next_scale;
                mean *= ratio;
                m2 = (m2 * ratio) * ratio;
                scale = next_scale;
            }
        }
        let value = value / scale;
        n += 1;
        let delta = value - mean;
        mean += delta / n as f64;
        m2 += delta * (value - mean);
        if min.is_none_or(|(min, _)| native < min) {
            min = Some((native, row));
        }
        if max.is_none_or(|(max, _)| native > max) {
            max = Some((native, row));
        }
    });
    if n > 0 {
        summary.mean = Some(mean * scale);
        summary.std_dev = Some(if n > 1 {
            // Restore units after the square root: variance itself may overflow
            // even when the standard deviation is representable.
            (m2 / (n - 1) as f64).sqrt() * scale
        } else {
            0.0
        });
    }
    NumericSummary {
        min_row: min.map(|(_, r)| r),
        max_row: max.map(|(_, r)| r),
        infinite_count,
        ..summary
    }
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
