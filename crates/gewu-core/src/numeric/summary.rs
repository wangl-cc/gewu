//! Native distinct values and extrema, with scalar conversion for mean and variance.

use std::{collections::HashSet, hash::Hash};

use arrow::{
    array::{Array, PrimitiveArray},
    datatypes::ArrowPrimitiveType,
};

use super::{IntegerColumn, RealColumn, RealType};
use crate::ColumnSummary;

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128> + TryFrom<i128> + Eq + Hash,
{
    pub(super) fn summarize(self, rows: impl Iterator<Item = usize>, summary: &mut ColumnSummary) {
        summarize(self.0, rows, |v| v.into() as f64, |v| v, summary);
    }
}

impl<'a, T: RealType> RealColumn<'a, T> {
    pub(super) fn summarize(self, rows: impl Iterator<Item = usize>, summary: &mut ColumnSummary) {
        summarize(
            self.array,
            rows,
            |v| T::as_float(v, self.scale),
            T::key,
            summary,
        );
    }
}

fn summarize<T, F, K, H>(
    array: &PrimitiveArray<T>,
    rows: impl Iterator<Item = usize>,
    as_float: F,
    key: K,
    summary: &mut ColumnSummary,
) where
    T: ArrowPrimitiveType,
    F: Fn(T::Native) -> f64,
    K: Fn(T::Native) -> H,
    H: Eq + Hash,
{
    let mut distinct = HashSet::new();
    let (mut n, mut mean, mut m2) = (0u64, 0.0, 0.0);
    let (mut min, mut max): (Option<T::Native>, Option<T::Native>) = (None, None);
    for row in rows {
        if array.is_null(row) {
            summary.null_count += 1;
            continue;
        }
        let native = array.value(row);
        let value = as_float(native);
        if value.is_nan() {
            summary.nan_count += 1;
            continue;
        }
        distinct.insert(key(native));
        n += 1;
        let delta = value - mean;
        mean += delta / n as f64;
        m2 += delta * (value - mean);
        if min.is_none_or(|min| native < min) {
            min = Some(native);
        }
        if max.is_none_or(|max| native > max) {
            max = Some(native);
        }
    }
    summary.distinct_count = distinct.len() as u64;
    summary.min = min.map(&as_float);
    summary.max = max.map(&as_float);
    if n > 0 {
        summary.mean = Some(mean);
        summary.std_dev = Some(if n > 1 {
            (m2 / (n - 1) as f64).sqrt()
        } else {
            0.0
        });
    }
}
