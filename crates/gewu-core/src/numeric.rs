//! Numeric view of a column, for range filters and histograms.
//!
//! Integer columns stay integers (`i64` or `u64`) so large ids keep their
//! exact value; floats and decimals become `f64`. Dates, times, timestamps
//! and durations use their integer physical value.

use arrow::array::{Array, ArrayRef, AsArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float64Type, Int64Type, UInt64Type};

use crate::{Bound, Error, Histogram, Result};

pub(crate) enum Numeric {
    Int(arrow::array::Int64Array),
    UInt(arrow::array::UInt64Array),
    Float(arrow::array::Float64Array),
}

impl Numeric {
    /// `None` when the column has no numeric meaning (text, bool, ...).
    pub(crate) fn build(array: &ArrayRef) -> Result<Option<Self>> {
        use DataType::*;
        let n = match array.data_type() {
            Int8 | Int16 | Int32 | Int64 => {
                Numeric::Int(cast(array, &Int64)?.as_primitive::<Int64Type>().clone())
            }
            UInt8 | UInt16 | UInt32 | UInt64 => {
                Numeric::UInt(cast(array, &UInt64)?.as_primitive::<UInt64Type>().clone())
            }
            Float16 | Float32 | Float64 | Decimal128(..) | Decimal256(..) => {
                Numeric::Float(cast(array, &Float64)?.as_primitive::<Float64Type>().clone())
            }
            Date32 | Time32(_) => {
                let as_i32 = cast(array, &Int32)?;
                Numeric::Int(cast(&as_i32, &Int64)?.as_primitive::<Int64Type>().clone())
            }
            Date64 | Time64(_) | Timestamp(..) | Duration(_) => {
                Numeric::Int(cast(array, &Int64)?.as_primitive::<Int64Type>().clone())
            }
            _ => return Ok(None),
        };
        Ok(Some(n))
    }

    fn is_valid(&self, r: usize) -> bool {
        match self {
            Numeric::Int(a) => a.is_valid(r),
            Numeric::UInt(a) => a.is_valid(r),
            Numeric::Float(a) => a.is_valid(r),
        }
    }

    /// The value at row `r` as a range bound. `None` for null.
    pub(crate) fn bound(&self, r: usize) -> Option<Bound> {
        if !self.is_valid(r) {
            return None;
        }
        Some(match self {
            Numeric::Int(a) => Bound::Int(a.value(r)),
            Numeric::UInt(a) => Bound::UInt(a.value(r)),
            Numeric::Float(a) => Bound::Float(a.value(r)),
        })
    }

    /// The value as `f64`, for binning. `None` for null.
    fn as_f64(&self, r: usize) -> Option<f64> {
        if !self.is_valid(r) {
            return None;
        }
        Some(match self {
            Numeric::Int(a) => a.value(r) as f64,
            Numeric::UInt(a) => a.value(r) as f64,
            Numeric::Float(a) => a.value(r),
        })
    }

    /// A per-row test for `min <= value <= max` (either bound optional).
    /// Nulls and NaN never pass.
    pub(crate) fn range_test(
        &self,
        min: Option<Bound>,
        max: Option<Bound>,
    ) -> Box<dyn Fn(usize) -> bool + Send + Sync> {
        let never: Box<dyn Fn(usize) -> bool + Send + Sync> = Box::new(|_| false);
        match self {
            Numeric::Int(a) => {
                let lo = match min {
                    None => i64::MIN,
                    Some(Bound::Int(x)) => x,
                    Some(Bound::UInt(x)) if x > i64::MAX as u64 => return never,
                    Some(Bound::UInt(x)) => x as i64,
                    Some(Bound::Float(f)) if f.is_nan() => return never,
                    // `as` saturates, so huge or infinite bounds clamp.
                    Some(Bound::Float(f)) => f.ceil() as i64,
                };
                let hi = match max {
                    None => i64::MAX,
                    Some(Bound::Int(x)) => x,
                    Some(Bound::UInt(x)) => x.min(i64::MAX as u64) as i64,
                    Some(Bound::Float(f)) if f.is_nan() => return never,
                    Some(Bound::Float(f)) => f.floor() as i64,
                };
                let a = a.clone();
                Box::new(move |r| a.is_valid(r) && (lo..=hi).contains(&a.value(r)))
            }
            Numeric::UInt(a) => {
                let lo = match min {
                    None => 0,
                    Some(Bound::Int(x)) => x.max(0) as u64,
                    Some(Bound::UInt(x)) => x,
                    Some(Bound::Float(f)) if f.is_nan() => return never,
                    Some(Bound::Float(f)) => f.ceil().max(0.0) as u64,
                };
                let hi = match max {
                    None => u64::MAX,
                    Some(Bound::Int(x)) if x < 0 => return never,
                    Some(Bound::Int(x)) => x as u64,
                    Some(Bound::UInt(x)) => x,
                    Some(Bound::Float(f)) if f.is_nan() || f < 0.0 => return never,
                    Some(Bound::Float(f)) => f.floor() as u64,
                };
                let a = a.clone();
                Box::new(move |r| a.is_valid(r) && (lo..=hi).contains(&a.value(r)))
            }
            Numeric::Float(a) => {
                let as_f = |b: Option<Bound>, default: f64| match b {
                    None => default,
                    Some(Bound::Int(x)) => x as f64,
                    Some(Bound::UInt(x)) => x as f64,
                    Some(Bound::Float(f)) => f,
                };
                let lo = as_f(min, f64::NEG_INFINITY);
                let hi = as_f(max, f64::INFINITY);
                if lo.is_nan() || hi.is_nan() {
                    return never;
                }
                let a = a.clone();
                // NaN fails both comparisons, so it never passes.
                Box::new(move |r| a.is_valid(r) && a.value(r) >= lo && a.value(r) <= hi)
            }
        }
    }

    /// Histogram of `rows`. The axis spans the whole column (so it doesn't
    /// jump as other filters change); counts cover only `rows`.
    pub(crate) fn histogram(
        &self,
        rows: impl Iterator<Item = usize>,
        bins: u32,
    ) -> Result<Histogram> {
        if bins == 0 {
            return Err(Error::InvalidArgument("bins must be at least 1".into()));
        }
        let integer = !matches!(self, Numeric::Float(_));

        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        let len = match self {
            Numeric::Int(a) => a.len(),
            Numeric::UInt(a) => a.len(),
            Numeric::Float(a) => a.len(),
        };
        for r in 0..len {
            if let Some(v) = self.as_f64(r).filter(|v| !v.is_nan()) {
                min = min.min(v);
                max = max.max(v);
            }
        }

        let mut h = Histogram {
            lo: None,
            hi: None,
            integer,
            counts: Vec::new(),
            null_count: 0,
            nan_count: 0,
            total: 0,
        };
        let has_values = min <= max;
        // Bins cover [lo, hi). For integers hi is max + 1, so every bin
        // holds whole numbers and a small span gets one bin per integer.
        // For floats, max falls in the last bin.
        let (lo, hi, n) = if !has_values {
            (0.0, 0.0, 0)
        } else if integer {
            let span = max - min + 1.0;
            (min, max + 1.0, span.min(bins as f64) as usize)
        } else if max > min {
            (min, max, bins as usize)
        } else {
            (min, min + 1.0, 1)
        };
        h.counts = vec![0; n];
        let width = if n > 0 { (hi - lo) / n as f64 } else { 0.0 };

        for r in rows {
            h.total += 1;
            match self.as_f64(r) {
                None => h.null_count += 1,
                Some(v) if v.is_nan() => h.nan_count += 1,
                Some(v) => {
                    let i = (((v - lo) / width) as usize).min(n - 1);
                    h.counts[i] += 1;
                }
            }
        }
        if has_values {
            h.lo = Some(lo);
            h.hi = Some(hi);
        }
        Ok(h)
    }
}
