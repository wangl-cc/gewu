//! Native distinct values and extrema, with scalar conversion for mean and variance.

use std::{collections::HashSet, hash::Hash};

use arrow::{
    array::{Array, PrimitiveArray},
    datatypes::ArrowPrimitiveType,
};

use super::{IntegerColumn, RealColumn, RealType};
use crate::{analysis::AnalysisContext, ColumnSummary, Result};

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128> + TryFrom<i128> + Eq + Hash,
{
    pub(super) fn summarize(
        self,
        rows: impl Iterator<Item = usize>,
        summary: &mut ColumnSummary,
        collect_distinct: bool,
        context: &AnalysisContext,
    ) -> Result<SummaryDetails> {
        summarize(
            self.0,
            rows,
            |v| v.into() as f64,
            |v| v,
            summary,
            collect_distinct,
            context,
        )
    }
}

impl<'a, T: RealType> RealColumn<'a, T> {
    pub(super) fn summarize(
        self,
        rows: impl Iterator<Item = usize>,
        summary: &mut ColumnSummary,
        collect_distinct: bool,
        context: &AnalysisContext,
    ) -> Result<SummaryDetails> {
        summarize(
            self.array,
            rows,
            |v| T::as_float(v, self.scale),
            T::key,
            summary,
            collect_distinct,
            context,
        )
    }
}

/// Rows holding the smallest and largest values (NaN left out).
pub(crate) struct SummaryDetails {
    pub min_row: Option<usize>,
    pub max_row: Option<usize>,
    pub infinite_count: u64,
}

fn summarize<T, F, K, H>(
    array: &PrimitiveArray<T>,
    rows: impl Iterator<Item = usize>,
    as_float: F,
    key: K,
    summary: &mut ColumnSummary,
    collect_distinct: bool,
    context: &AnalysisContext,
) -> Result<SummaryDetails>
where
    T: ArrowPrimitiveType,
    F: Fn(T::Native) -> f64,
    K: Fn(T::Native) -> H,
    H: Eq + Hash,
{
    context.check()?;
    let mut distinct = collect_distinct.then(HashSet::new);
    let mut infinite_count = 0;
    let (mut n, mut mean, mut m2) = (0u64, 0.0, 0.0);
    // (value, row) of the smallest and largest so far.
    let mut min: Option<(T::Native, usize)> = None;
    let mut max: Option<(T::Native, usize)> = None;
    for (i, row) in rows.enumerate() {
        context.check_at(i)?;
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
        if let Some(distinct) = &mut distinct {
            distinct.insert(key(native));
        }
        infinite_count += u64::from(value.is_infinite());
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
    }
    summary.distinct_count = distinct.map_or(0, |values| values.len() as u64);
    summary.min = min.map(|(v, _)| as_float(v));
    summary.max = max.map(|(v, _)| as_float(v));
    if n > 0 {
        summary.mean = Some(mean);
        summary.std_dev = Some(if n > 1 {
            (m2 / (n - 1) as f64).sqrt()
        } else {
            0.0
        });
    }
    context.check()?;
    Ok(SummaryDetails {
        min_row: min.map(|(_, r)| r),
        max_row: max.map(|(_, r)| r),
        infinite_count,
    })
}
