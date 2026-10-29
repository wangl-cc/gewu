//! Filter histograms retain exact typed bin bounds. Approximate axis values
//! are display-only; counting and applying a selected bin use the same cuts.

use arrow::{
    array::{
        downcast_integer_array, downcast_temporal_array, Array, ArrayRef, AsArray, PrimitiveArray,
    },
    datatypes::*,
};

use crate::{scope::Selection, Bound, Error, Histogram, RangeBounds, RangeHistogram, Result};

/// Range-filter charts have a bounded number of options, independent of row count.
const MAX_BINS: u32 = 60;

pub(crate) fn build(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
) -> Result<Option<RangeHistogram>> {
    if bins == 0 {
        return Err(Error::InvalidArgument("bins must be at least 1".into()));
    }
    let bins = bins.min(MAX_BINS) as usize;
    match array.data_type() {
        data_type if data_type.is_integer() => {
            downcast_integer_array!(array => integer(array, rows, bins), _ => unreachable!("integer dispatch"))
        }
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(..)
        | DataType::Duration(_) => {
            downcast_temporal_array!(array => integer(array, rows, bins), _ => Ok(None))
        }
        DataType::Float16 => floating(array.as_primitive::<Float16Type>(), rows, bins),
        DataType::Float32 => floating(array.as_primitive::<Float32Type>(), rows, bins),
        DataType::Float64 => floating(array.as_primitive::<Float64Type>(), rows, bins),
        DataType::Decimal32(_, scale) => decimal(
            array.as_primitive::<Decimal32Type>(),
            rows,
            bins,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i32,
        ),
        DataType::Decimal64(_, scale) => decimal(
            array.as_primitive::<Decimal64Type>(),
            rows,
            bins,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i64,
        ),
        DataType::Decimal128(_, scale) => decimal(
            array.as_primitive::<Decimal128Type>(),
            rows,
            bins,
            *scale,
            i256::from_i128,
            |value| value.to_i128().expect("native coefficient domain"),
        ),
        DataType::Decimal256(_, scale) => decimal(
            array.as_primitive::<Decimal256Type>(),
            rows,
            bins,
            *scale,
            |value| value,
            |value| value,
        ),
        // Dictionaries still support typed text ranges, but have no chart until
        // their encoded values can be counted without decoding the source column.
        _ => Ok(None),
    }
}

fn extrema<T: ArrowPrimitiveType>(array: &PrimitiveArray<T>) -> Option<(T::Native, T::Native)>
where
    T::Native: Ord,
{
    let mut values = array.iter().flatten();
    let first = values.next()?;
    Some(values.fold((first, first), |(lo, hi), value| {
        (lo.min(value), hi.max(value))
    }))
}

fn integer<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
) -> Result<Option<RangeHistogram>>
where
    T::Native: Into<i128> + Ord,
{
    let Some((min, max)) = extrema(array) else {
        return Ok(None);
    };
    let (min, max) = (min.into(), max.into());
    // Every supported integer/temporal array fits i64 or u64, so this inclusive
    // domain and the small interpolation products fit comfortably in i128.
    let span = max - min + 1;
    let n = span.min(bins as i128) as usize;
    let cuts = (1..n)
        .map(|i| min + (span * i as i128 + n as i128 - 1) / n as i128)
        .collect::<Vec<_>>();
    let bound = |value: i128| {
        if array.data_type().is_unsigned_integer() {
            Bound::from_u64(array.data_type(), value as u64)
        } else {
            Bound::from_i64(array.data_type(), value as i64)
        }
    };
    let ranges = bin_ranges(&cuts, bound, |value| bound(value - 1))?;
    let min_label = bound(min)?.display().to_owned();
    let max_label = bound(max)?.display().to_owned();
    let histogram = count(
        array,
        rows,
        n,
        true,
        axis(min as f64, (max + 1) as f64, n),
        |value| Some(cuts.partition_point(|&cut| cut <= value.into())),
    );
    Ok(Some(RangeHistogram {
        histogram,
        ranges,
        min_label,
        max_label,
    }))
}

fn decimal<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    scale: i8,
    widen: impl Fn(T::Native) -> i256,
    narrow: impl Fn(i256) -> T::Native,
) -> Result<Option<RangeHistogram>>
where
    T::Native: Ord,
{
    let Some((min, max)) = extrema(array) else {
        return Ok(None);
    };
    let (min, max) = (widen(min), widen(max));
    // Malformed arrays can hold coefficients outside their declared decimal
    // domain. If their span exceeds i256, omit the chart instead of rounding it.
    let Some(span) = max
        .checked_sub(min)
        .and_then(|span| span.checked_add(i256::ONE))
    else {
        return Ok(None);
    };
    let n = span
        .to_i128()
        .map_or(bins, |span| span.min(bins as i128) as usize);
    let divisor = i256::from_i128(n as i128);
    let quotient = span / divisor;
    let remainder = (span % divisor)
        .to_i128()
        .expect("remainder is smaller than the bin count");
    let cuts = (1..n)
        .map(|i| {
            // ceil(span * i / n), without overflowing the wide coefficient type.
            min + quotient * i256::from_i128(i as i128)
                + i256::from_i128((remainder * i as i128 + n as i128 - 1) / n as i128)
        })
        .collect::<Vec<_>>();
    let bound = |coefficient| Bound::from_decimal(array.data_type(), coefficient, scale);
    let ranges = bin_ranges(&cuts, bound, |coefficient| bound(coefficient - i256::ONE))?;
    let (lower, upper) = (bound(min)?, bound(max)?);
    let (lo, hi) = (lower.axis_value(), upper.axis_value());
    let min_label = lower.display().to_owned();
    let max_label = upper.display().to_owned();
    let native_cuts = cuts.into_iter().map(narrow).collect::<Vec<_>>();
    let histogram = count(array, rows, n, false, axis(lo, hi, n), |value| {
        Some(native_cuts.partition_point(|&cut| cut <= value))
    });
    Ok(Some(RangeHistogram {
        histogram,
        ranges,
        min_label,
        max_label,
    }))
}

fn floating<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
) -> Result<Option<RangeHistogram>>
where
    T::Native: Into<f64>,
{
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in array
        .iter()
        .flatten()
        .map(Into::into)
        .filter(|v: &f64| v.is_finite())
    {
        min = min.min(value);
        max = max.max(value);
    }
    if min > max {
        return Ok(None);
    }
    let mut cuts = Vec::with_capacity(bins - 1);
    if min < max {
        let span = max - min;
        for i in 1..bins {
            let fraction = i as f64 / bins as f64;
            let cut = if span.is_finite() {
                min + span * fraction
            } else {
                min * (1.0 - fraction) + max * fraction
            };
            // Nearby representable values can collapse several interpolated
            // cuts. Drop those cuts instead of creating reversed/empty ranges.
            if cut > min && cut <= max && cuts.last().is_none_or(|previous| *previous < cut) {
                cuts.push(cut);
            }
        }
    }
    let bound = |value| Bound::from_f64(array.data_type(), value);
    let ranges = bin_ranges(&cuts, bound, |value: f64| bound(value.next_down()))?;
    let n = ranges.len();
    let min_label = bound(min)?.display().to_owned();
    let max_label = bound(max)?.display().to_owned();
    let histogram = count(array, rows, n, false, axis(min, max, n), |value| {
        let value: f64 = value.into();
        (!value.is_nan()).then(|| cuts.partition_point(|&cut| cut <= value))
    });
    Ok(Some(RangeHistogram {
        histogram,
        ranges,
        min_label,
        max_label,
    }))
}

fn bin_ranges<C: Copy>(
    cuts: &[C],
    lower: impl Fn(C) -> Result<Bound>,
    upper: impl Fn(C) -> Result<Bound>,
) -> Result<Vec<RangeBounds>> {
    (0..=cuts.len())
        .map(|i| {
            Ok(RangeBounds {
                min: if i == 0 {
                    None
                } else {
                    Some(lower(cuts[i - 1])?)
                },
                max: cuts.get(i).copied().map(&upper).transpose()?,
            })
        })
        .collect()
}

/// Keep the plotting axis finite and nonempty even when exact adjacent
/// coefficients share a floating representation. Bin selection uses indices.
fn axis(lo: f64, hi: f64, bins: usize) -> (f64, f64) {
    if lo.is_finite() && hi.is_finite() && lo < hi {
        return (lo, hi);
    }
    let expanded = (lo.next_down(), hi.next_up());
    if expanded.0.is_finite() && expanded.1.is_finite() && expanded.0 < expanded.1 {
        expanded
    } else {
        (0.0, bins as f64)
    }
}

fn count<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    integer: bool,
    axis: (f64, f64),
    classify: impl Fn(T::Native) -> Option<usize>,
) -> Histogram {
    let mut histogram = Histogram {
        lo: Some(axis.0),
        hi: Some(axis.1),
        integer,
        counts: vec![0; bins],
        null_count: 0,
        nan_count: 0,
        total: rows.len() as u64,
    };
    for row in rows.iter() {
        if array.is_null(row) {
            histogram.null_count += 1;
        } else if let Some(bin) = classify(array.value(row)) {
            histogram.counts[bin] += 1;
        } else {
            histogram.nan_count += 1;
        }
    }
    histogram
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        Date32Array, Decimal128Array, Decimal256Array, DictionaryArray, Float32Array, Float64Array,
        Int64Array, Int8Array, Time32MillisecondArray, TimestampNanosecondArray, UInt64Array,
    };

    use super::*;
    use crate::{column::Column, engine::CancelToken, Filter};

    fn verify_bins(array: ArrayRef, rows: Selection, bins: u32) -> RangeHistogram {
        let result = build(&array, &rows, bins).unwrap().unwrap();
        assert_eq!(result.ranges.len(), result.histogram.counts.len());
        assert!(result.ranges.first().unwrap().min.is_none());
        assert!(result.ranges.last().unwrap().max.is_none());
        let mut selected = vec![0_u32; array.len()];
        for (bounds, &expected) in result.ranges.iter().zip(&result.histogram.counts) {
            let predicate = Column::new(&array)
                .prepare_filter(&Filter::Range {
                    column: 0,
                    min: bounds.min.clone(),
                    max: bounds.max.clone(),
                })
                .unwrap();
            let mut matched = vec![false; array.len()];
            predicate
                .evaluate(0..array.len(), &mut matched, &CancelToken::never())
                .unwrap();
            assert_eq!(
                rows.iter().filter(|&row| matched[row]).count() as u64,
                expected
            );
            for (selected, matched) in selected.iter_mut().zip(matched) {
                *selected += u32::from(matched);
            }
        }
        assert!(
            selected.iter().all(|&count| count <= 1),
            "bin ranges overlap"
        );
        assert_eq!(result.histogram.total, rows.len() as u64);
        assert_eq!(
            result.histogram.counts.iter().sum::<u64>()
                + result.histogram.null_count
                + result.histogram.nan_count,
            result.histogram.total
        );
        result
    }

    #[test]
    fn integer_and_temporal_bins_keep_exact_edges_and_facet_counts() {
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(vec![
                Some(i64::MIN),
                Some(9_007_199_254_740_992),
                Some(9_007_199_254_740_993),
                Some(i64::MAX),
                None,
            ])),
            Arc::new(UInt64Array::from(vec![
                Some(u64::MAX - 2),
                Some(u64::MAX - 1),
                Some(u64::MAX),
                None,
            ])),
            Arc::new(Date32Array::from(vec![Some(0), Some(1), Some(2), None])),
            Arc::new(Time32MillisecondArray::from(vec![
                Some(0),
                Some(1),
                Some(2),
                None,
            ])),
            Arc::new(
                TimestampNanosecondArray::from(vec![
                    Some(9_007_199_254_740_992),
                    Some(9_007_199_254_740_993),
                    Some(9_007_199_254_740_994),
                    None,
                ])
                .with_timezone("UTC"),
            ),
        ];
        for array in arrays {
            let len = array.len();
            verify_bins(array.clone(), Selection::All(len), 60);
            verify_bins(
                array,
                Selection::Rows(Arc::new(vec![1, (len - 1) as u32])),
                3,
            );
        }
        let adjacent: ArrayRef = Arc::new(Int64Array::from(vec![
            9_007_199_254_740_992,
            9_007_199_254_740_993,
        ]));
        let result = verify_bins(adjacent, Selection::All(2), 60);
        assert_eq!(result.histogram.counts, [1, 1]);
        assert!(result.histogram.lo.unwrap() < result.histogram.hi.unwrap());
        assert_eq!(result.min_label, "9007199254740992");
        assert_eq!(result.max_label, "9007199254740993");
    }

    #[test]
    fn decimal_bins_preserve_large_coefficients_and_negative_scales() {
        let large: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(
                Decimal128Array::from(vec![
                    9_007_199_254_740_992,
                    9_007_199_254_740_993,
                    9_007_199_254_740_994,
                ])
                .with_precision_and_scale(38, 2)
                .unwrap(),
            ),
            Arc::new(
                Decimal128Array::from(vec![-1, 0, 1])
                    .with_precision_and_scale(20, -2)
                    .unwrap(),
            ),
            Arc::new(
                Decimal256Array::from(vec![large, large + i256::ONE, large + i256::from(2)])
                    .with_precision_and_scale(76, 4)
                    .unwrap(),
            ),
        ];
        for array in arrays {
            let result = verify_bins(array, Selection::All(3), 60);
            assert_eq!(result.histogram.counts, [1, 1, 1]);
        }
        let limit: i256 =
            "9999999999999999999999999999999999999999999999999999999999999999999999999999"
                .parse()
                .unwrap();
        let extremes: ArrayRef = Arc::new(
            Decimal256Array::from(vec![-limit, i256::ZERO, limit])
                .with_precision_and_scale(76, 0)
                .unwrap(),
        );
        verify_bins(extremes, Selection::All(3), 60);
    }

    #[test]
    fn float_bins_share_boundary_semantics_and_separate_null_nan_infinity() {
        let float32: ArrayRef = Arc::new(Float32Array::from(vec![
            Some(f32::NEG_INFINITY),
            Some(0.1),
            Some(0.2),
            Some(1.0),
            Some(f32::INFINITY),
            Some(f32::NAN),
            None,
        ]));
        let float16 = arrow::compute::cast(&float32, &DataType::Float16).unwrap();
        verify_bins(float16, Selection::All(7), 3);
        let result = verify_bins(float32, Selection::All(7), 3);
        assert_eq!(
            (result.histogram.null_count, result.histogram.nan_count),
            (1, 1)
        );
        for values in [
            vec![-f64::MAX, 0.0, f64::MAX],
            vec![1.0, 1.0_f64.next_up()],
            vec![0.0, 0.0_f64.next_up()],
            vec![f64::MAX],
        ] {
            let len = values.len();
            let single = values.len() == 1;
            let result = verify_bins(
                Arc::new(Float64Array::from(values)),
                Selection::All(len),
                60,
            );
            if single {
                assert_eq!(result.min_label, f64::MAX.to_string());
                assert_eq!(result.max_label, f64::MAX.to_string());
            }
        }
    }

    #[test]
    fn omitted_charts_and_empty_facets_are_explicit() {
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(vec![None])),
            Arc::new(Float64Array::from(vec![f64::NAN])),
            Arc::new(Float64Array::from(vec![f64::INFINITY])),
            Arc::new(
                DictionaryArray::<Int8Type>::try_new(
                    Int8Array::from(vec![0]),
                    Arc::new(Int64Array::from(vec![1])),
                )
                .unwrap(),
            ),
        ];
        for array in arrays {
            assert!(build(&array, &Selection::All(1), 60).unwrap().is_none());
        }
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(0..100));
        assert!(build(&array, &Selection::All(100), 0).is_err());
        let result = verify_bins(array, Selection::Rows(Arc::new(vec![])), 1000);
        assert_eq!(result.ranges.len(), MAX_BINS as usize);
        assert!(result.histogram.counts.iter().all(|&count| count == 0));
    }
}
