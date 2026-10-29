//! Native integer and floating-point binning over a selected set of source rows.

use std::hash::Hash;

use arrow::{array::Array, datatypes::ArrowPrimitiveType};

use super::{IntegerColumn, RealColumn, RealType};
use crate::{Error, Histogram, Result};

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128> + TryFrom<i128> + Eq + Hash,
{
    pub(super) fn histogram(
        self,
        rows: impl Iterator<Item = usize>,
        bins: u32,
    ) -> Result<Histogram> {
        let mut min = i128::MAX;
        let mut max = i128::MIN;
        for value in self.0.iter().flatten() {
            let value = value.into();
            min = min.min(value);
            max = max.max(value);
        }
        let mut h = empty_histogram(true);
        if min <= max {
            let span = max - min + 1;
            let n = span.min(i128::from(bins)) as usize;
            h.counts = vec![0; n];
            // The public display axis is f64, but bin membership is computed
            // exactly in i128, including adjacent integers above 2^53.
            let mut lo = min as f64;
            let mut hi = (max + 1) as f64;
            if lo as i128 > min {
                lo = lo.next_down();
            }
            if (hi as i128) < max + 1 {
                hi = hi.next_up();
            }
            h.lo = Some(lo);
            h.hi = Some(hi);
            for row in rows {
                h.total += 1;
                if self.0.is_null(row) {
                    h.null_count += 1;
                } else {
                    let offset = self.0.value(row).into() - min;
                    let bin = (offset * n as i128 / span) as usize;
                    h.counts[bin] += 1;
                }
            }
        } else {
            h.total = rows.count() as u64;
            h.null_count = h.total;
        }
        Ok(h)
    }
}

impl<'a, T: RealType> RealColumn<'a, T> {
    pub(super) fn histogram(
        self,
        rows: impl Iterator<Item = usize>,
        bins: u32,
    ) -> Result<Histogram> {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut has_infinite = false;
        for value in self
            .array
            .iter()
            .flatten()
            .map(|v| T::as_float(v, self.scale))
        {
            if value.is_finite() {
                min = min.min(value);
                max = max.max(value);
            } else if value.is_infinite() {
                has_infinite = true;
            }
        }
        let mut h = empty_histogram(false);
        if min > max && has_infinite {
            return Err(Error::InvalidArgument(
                "histogram needs at least one finite value".into(),
            ));
        }
        if min <= max {
            let n = if min == max { 1 } else { bins as usize };
            h.counts = vec![0; n];
            h.lo = Some(min);
            // Preserve the usual one-value axis; next_up handles values for
            // which adding one rounds away. f64::MAX has no finite successor.
            h.hi = Some(if min == max {
                let hi = (min + 1.0).max(min.next_up());
                if hi.is_finite() {
                    hi
                } else {
                    min
                }
            } else {
                max
            });
        }
        for row in rows {
            h.total += 1;
            if self.array.is_null(row) {
                h.null_count += 1;
                continue;
            }
            let value = T::as_float(self.array.value(row), self.scale);
            if value.is_nan() {
                h.nan_count += 1;
                continue;
            }
            let n = h.counts.len();
            let bin = if n == 1 || value <= min {
                0
            } else if value >= max {
                n - 1
            } else {
                // Halving avoids overflow when finite endpoints span nearly
                // the full f64 range. Use the direct ratio for narrow spans,
                // including subnormals, where halving could lose precision.
                let span = max - min;
                let fraction = if span.is_finite() {
                    (value - min) / span
                } else {
                    (value * 0.5 - min * 0.5) / (max * 0.5 - min * 0.5)
                };
                ((fraction * n as f64) as usize).min(n - 1)
            };
            h.counts[bin] += 1;
        }
        Ok(h)
    }
}

fn empty_histogram(integer: bool) -> Histogram {
    Histogram {
        lo: None,
        hi: None,
        integer,
        counts: Vec::new(),
        null_count: 0,
        nan_count: 0,
        total: 0,
    }
}
