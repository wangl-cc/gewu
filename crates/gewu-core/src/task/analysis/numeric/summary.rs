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
    let mut scale = f64::MIN_POSITIVE;
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
    }
    summary.distinct_count = distinct.map_or(0, |values| values.len() as u64);
    summary.min = min.map(|(v, _)| as_float(v));
    summary.max = max.map(|(v, _)| as_float(v));
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
    context.check()?;
    Ok(SummaryDetails {
        min_row: min.map(|(_, r)| r),
        max_row: max.map(|(_, r)| r),
        infinite_count,
    })
}

#[cfg(test)]
mod tests {
    use arrow::array::Float64Array;

    use super::*;

    fn summary(values: &[f64]) -> ColumnSummary {
        let array = Float64Array::from(values.to_vec());
        let mut result = ColumnSummary::default();
        RealColumn::new(&array)
            .summarize(
                0..values.len(),
                &mut result,
                false,
                &AnalysisContext::default(),
            )
            .unwrap();
        result
    }

    #[test]
    fn moments_preserve_representable_results_across_float_scales() {
        let large = 2.0f64.powi(1000);
        let ulp = 2.0f64.powi(948);
        let tiny = f64::from_bits(1);
        for (values, mean, deviation) in [
            (vec![1e308, -1e308], 0.0, 1e308 * 2.0f64.sqrt()),
            (vec![1e200, -1e200], 0.0, 1e200 * 2.0f64.sqrt()),
            (vec![0.0, 1e200, -1e200], 0.0, 1e200),
            (
                vec![large, large + 4.0 * ulp],
                large + 2.0 * ulp,
                2.0f64.sqrt() * 2.0 * ulp,
            ),
            (vec![f64::MAX, f64::MAX], f64::MAX, 0.0),
            (vec![tiny, 3.0 * tiny], 2.0 * tiny, tiny),
        ] {
            let result = summary(&values);
            assert_eq!(result.mean, Some(mean), "{values:?}");
            let actual = result.std_dev.unwrap();
            assert!(actual.is_finite(), "{values:?}: {actual}");
            assert!(
                (actual - deviation).abs() <= deviation * 1e-14,
                "{values:?}: {actual} != {deviation}"
            );
        }
    }

    #[test]
    fn moments_keep_nan_exclusion_and_infinity_propagation() {
        let result = summary(&[1.0, f64::NAN, 3.0]);
        assert_eq!((result.nan_count, result.mean), (1, Some(2.0)));
        assert_eq!(result.std_dev, Some(2.0f64.sqrt()));
        let result = summary(&[f64::INFINITY]);
        assert_eq!(
            (result.mean, result.std_dev),
            (Some(f64::INFINITY), Some(0.0))
        );
        let result = summary(&[1.0, f64::INFINITY]);
        assert_eq!(result.mean, Some(f64::INFINITY));
        assert!(result.std_dev.unwrap().is_nan());
        let result = summary(&[f64::INFINITY, 1.0]);
        assert!(result.mean.unwrap().is_nan());
        assert!(result.std_dev.unwrap().is_nan());
    }
}
