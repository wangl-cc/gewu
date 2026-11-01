//! Geometric cuts are prepared once, then native values decide bin membership.
//! Logarithms affect cut placement and plotting only, never filter comparison.

use arrow::{
    array::{downcast_integer_array, Array, ArrayRef, AsArray, PrimitiveArray},
    datatypes::*,
};
use num_traits::ToPrimitive;

use super::{Histogram, Output, RangeBounds};
use crate::{
    analysis::AnalysisContext, scope::Selection, AxisScale, Bound, Error, LogicalType, Result,
};

pub(super) fn build<H: Output>(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    context: &AnalysisContext,
) -> Result<H> {
    context.check()?;
    if bins == 0 {
        return Err(Error::InvalidArgument("bins must be at least 1".into()));
    }
    if !LogicalType::from_arrow(array.data_type()).supports_logarithmic() {
        return Err(Error::InvalidArgument(format!(
            "{} does not support logarithmic histograms",
            array.data_type()
        )));
    }
    let bins = bins as usize;
    match array.data_type() {
        t if t.is_integer() => {
            downcast_integer_array!(array => integer(array, rows, bins, context), _ => unreachable!("integer dispatch"))
        }
        DataType::Float16 => floating(array.as_primitive::<Float16Type>(), rows, bins, context),
        DataType::Float32 => floating(array.as_primitive::<Float32Type>(), rows, bins, context),
        DataType::Float64 => floating(array.as_primitive::<Float64Type>(), rows, bins, context),
        DataType::Decimal32(_, scale) => decimal(
            array.as_primitive::<Decimal32Type>(),
            rows,
            bins,
            context,
            *scale,
            |v| i256::from_i128(i128::from(v)),
            |v| v.to_i128().expect("native coefficient domain") as i32,
        ),
        DataType::Decimal64(_, scale) => decimal(
            array.as_primitive::<Decimal64Type>(),
            rows,
            bins,
            context,
            *scale,
            |v| i256::from_i128(i128::from(v)),
            |v| v.to_i128().expect("native coefficient domain") as i64,
        ),
        DataType::Decimal128(_, scale) => decimal(
            array.as_primitive::<Decimal128Type>(),
            rows,
            bins,
            context,
            *scale,
            i256::from_i128,
            |v| v.to_i128().expect("native coefficient domain"),
        ),
        DataType::Decimal256(_, scale) => decimal(
            array.as_primitive::<Decimal256Type>(),
            rows,
            bins,
            context,
            *scale,
            |v| v,
            |v| v,
        ),
        _ => unreachable!("validated logarithmic type"),
    }
}

fn positive_extrema<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    context: &AnalysisContext,
    positive: impl Fn(T::Native) -> bool,
) -> Result<Option<(T::Native, T::Native)>>
where
    T::Native: PartialOrd,
{
    let mut extrema: Option<(T::Native, T::Native)> = None;
    for (index, value) in array.iter().enumerate() {
        context.check_at(index)?;
        if let Some(value) = value.filter(|&v| positive(v)) {
            extrema = Some(match extrema {
                Some((lo, hi)) => (
                    if value < lo { value } else { lo },
                    if value > hi { value } else { hi },
                ),
                None => (value, value),
            });
        }
    }
    context.check()?;
    Ok(extrema)
}

/// The native grid can collapse adjacent geometric cuts. Keep only increasing
/// cuts in the source domain rather than create empty or reversed ranges.
fn cuts<N: Copy + PartialOrd>(
    min: N,
    max: N,
    log_min: f64,
    log_max: f64,
    bins: usize,
    context: &AnalysisContext,
    native: impl Fn(f64) -> Option<N>,
) -> Result<Vec<N>> {
    let mut cuts = Vec::with_capacity(bins.saturating_sub(1));
    if log_min < log_max {
        for index in 1..bins {
            context.check_at(index)?;
            let exponent = log_min + (log_max - log_min) * (index as f64 / bins as f64);
            let Some(cut) = native(10_f64.powf(exponent)) else {
                continue;
            };
            if cut > min && cut <= max && cuts.last().is_none_or(|&last| last < cut) {
                cuts.push(cut);
            }
        }
    }
    Ok(cuts)
}

fn integer<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
) -> Result<H>
where
    T::Native: Into<i128> + TryFrom<i128> + Ord,
{
    let extrema = positive_extrema(array, context, |value| value.into() > 0)?;
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, rows, empty(rows), context, |_| Class::NonPositive)?,
            || Ok(Vec::new()),
        );
    };
    let (lo, hi) = ((min.into() as f64).log10(), (max.into() as f64).log10());
    let cuts = cuts(min, max, lo, hi, bins, context, |value| {
        T::Native::try_from(value.ceil() as i128).ok()
    })?;
    let bound = |value: T::Native| {
        if array.data_type().is_unsigned_integer() {
            Bound::from_u64(array.data_type(), value.into() as u64)
        } else {
            Bound::from_i64(array.data_type(), value.into() as i64)
        }
    };
    let lower = bound(min)?;
    let upper = bound(max)?;
    let edges = plot_edges(
        lo,
        ((max.into() + 1) as f64).log10(),
        cuts.iter().map(|&value| (value.into() as f64).log10()),
        context,
    )?;
    let histogram = count(
        array,
        rows,
        with_domain(rows, edges, &lower, &upper),
        context,
        |value| {
            if value.into() <= 0 {
                Class::NonPositive
            } else {
                Class::Bin(cuts.partition_point(|&cut| cut <= value))
            }
        },
    )?;
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, context, bound, |value| {
            let previous = T::Native::try_from(value.into() - 1)
                .ok()
                .expect("positive cut predecessor");
            bound(previous)
        })
    })
}

fn decimal<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
    scale: i8,
    widen: impl Fn(T::Native) -> i256,
    narrow: impl Fn(i256) -> T::Native,
) -> Result<H>
where
    T::Native: Ord,
{
    let extrema = positive_extrema(array, context, |value| widen(value) > i256::ZERO)?;
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, rows, empty(rows), context, |_| Class::NonPositive)?,
            || Ok(Vec::new()),
        );
    };
    let (min_coefficient, max_coefficient) = (widen(min), widen(max));
    // Work in coefficient space: the decimal scale only shifts the log axis.
    // Multiplying by 10^-scale first could lose a representable logarithm.
    let lo = min_coefficient
        .to_f64()
        .expect("i256 exponent fits f64")
        .log10();
    let hi = max_coefficient
        .to_f64()
        .expect("i256 exponent fits f64")
        .log10();
    let cuts = cuts(min, max, lo, hi, bins, context, |value| {
        let coefficient = i256::from_f64(value.ceil())?;
        (coefficient > min_coefficient && coefficient <= max_coefficient)
            .then(|| narrow(coefficient))
    })?;
    let bound = |value| Bound::from_decimal(array.data_type(), widen(value), scale);
    let lower = bound(min)?;
    let upper = bound(max)?;
    let log_value = |coefficient: i256| {
        coefficient
            .to_f64()
            .expect("i256 exponent fits f64")
            .log10()
            - f64::from(scale)
    };
    let edges = plot_edges(
        lo - f64::from(scale),
        log_value(
            max_coefficient
                .checked_add(i256::ONE)
                .unwrap_or(max_coefficient),
        ),
        cuts.iter().map(|&value| log_value(widen(value))),
        context,
    )?;
    let histogram = count(
        array,
        rows,
        with_domain(rows, edges, &lower, &upper),
        context,
        |value| {
            if widen(value) <= i256::ZERO {
                Class::NonPositive
            } else {
                Class::Bin(cuts.partition_point(|&cut| cut <= value))
            }
        },
    )?;
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, context, bound, |value| {
            Bound::from_decimal(array.data_type(), widen(value) - i256::ONE, scale)
        })
    })
}

fn floating<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
) -> Result<H>
where
    T::Native: Into<f64> + PartialOrd,
{
    let extrema = positive_extrema(array, context, |value| {
        let value = value.into();
        value > 0.0 && value.is_finite()
    })?;
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, rows, empty(rows), context, |value| {
                classify_float(value.into(), &[])
            })?,
            || Ok(Vec::new()),
        );
    };
    let (min, max) = (min.into(), max.into());
    let (lo, hi) = (min.log10(), max.log10());
    let cuts = cuts(min, max, lo, hi, bins, context, Some)?;
    let bound = |value| Bound::from_f64(array.data_type(), value);
    let lower = bound(min)?;
    let upper = bound(max)?;
    // Include a final cut that rounds to max without giving its singleton bin
    // zero width. At f64::MAX expand in log space to keep coordinates finite.
    let display_hi = if max < f64::MAX {
        max.next_up().log10()
    } else {
        hi.next_up()
    };
    let edges = plot_edges(
        lo,
        display_hi,
        cuts.iter().map(|value| value.log10()),
        context,
    )?;
    let histogram = count(
        array,
        rows,
        with_domain(rows, edges, &lower, &upper),
        context,
        |value| classify_float(value.into(), &cuts),
    )?;
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, context, bound, |value| {
            bound(value.next_down())
        })
    })
}

enum Class {
    Bin(usize),
    NonPositive,
    Infinite,
    NaN,
}

fn classify_float(value: f64, cuts: &[f64]) -> Class {
    if value.is_nan() {
        Class::NaN
    } else if value.is_infinite() {
        Class::Infinite
    } else if value <= 0.0 {
        Class::NonPositive
    } else {
        Class::Bin(cuts.partition_point(|&cut| cut <= value))
    }
}

fn empty(rows: &Selection) -> Histogram {
    Histogram {
        scale: AxisScale::Log10,
        lo: None,
        hi: None,
        min_label: None,
        max_label: None,
        integer: false,
        counts: Vec::new(),
        bin_edges: Vec::new(),
        null_count: 0,
        nan_count: 0,
        non_positive_count: 0,
        infinite_count: 0,
        total: rows.len() as u64,
    }
}

/// Preserve the unequal widths left by rounding geometric cuts onto a native
/// grid. Adjacent exact values can share a logarithm at f64 precision; nudge
/// only those display coordinates, never the native cut or filter endpoint.
fn plot_edges(
    lo: f64,
    hi: f64,
    cuts: impl ExactSizeIterator<Item = f64>,
    context: &AnalysisContext,
) -> Result<Vec<f64>> {
    let mut edges = Vec::with_capacity(cuts.len() + 2);
    edges.push(lo);
    let mut previous = lo;
    for (index, edge) in cuts.chain(std::iter::once(hi)).enumerate() {
        context.check_at(index)?;
        let edge = if edge <= previous {
            previous.next_up()
        } else {
            edge
        };
        edges.push(edge);
        previous = edge;
    }
    Ok(edges)
}

fn with_domain(rows: &Selection, bin_edges: Vec<f64>, min: &Bound, max: &Bound) -> Histogram {
    Histogram {
        lo: bin_edges.first().copied(),
        hi: bin_edges.last().copied(),
        min_label: Some(min.display().to_owned()),
        max_label: Some(max.display().to_owned()),
        counts: vec![0; bin_edges.len() - 1],
        bin_edges,
        ..empty(rows)
    }
}

fn count<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    mut histogram: Histogram,
    context: &AnalysisContext,
    classify: impl Fn(T::Native) -> Class,
) -> Result<Histogram> {
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if array.is_null(row) {
            histogram.null_count += 1;
        } else {
            match classify(array.value(row)) {
                Class::Bin(bin) => histogram.counts[bin] += 1,
                Class::NonPositive => histogram.non_positive_count += 1,
                Class::Infinite => histogram.infinite_count += 1,
                Class::NaN => histogram.nan_count += 1,
            }
        }
    }
    context.check()?;
    Ok(histogram)
}

fn finite_ranges<N: Copy>(
    cuts: &[N],
    min: Bound,
    max: Bound,
    context: &AnalysisContext,
    lower: impl Fn(N) -> Result<Bound>,
    upper: impl Fn(N) -> Result<Bound>,
) -> Result<Vec<RangeBounds>> {
    (0..=cuts.len())
        .map(|index| {
            context.check_at(index)?;
            Ok(RangeBounds {
                min: Some(if index == 0 {
                    min.clone()
                } else {
                    lower(cuts[index - 1])?
                }),
                max: Some(if index == cuts.len() {
                    max.clone()
                } else {
                    upper(cuts[index])?
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        Date32Array, Decimal128Array, Decimal256Array, Float32Array, Float64Array, Int64Array,
        UInt64Array,
    };

    use super::*;
    use crate::{column::Column, engine::CancelToken, Filter, RangeHistogram};

    fn verify(array: ArrayRef, rows: Selection, bins: u32) -> Histogram {
        let result: RangeHistogram =
            build(&array, &rows, bins, &AnalysisContext::default()).unwrap();
        let histogram: Histogram = build(&array, &rows, bins, &AnalysisContext::default()).unwrap();
        assert_eq!(histogram, result.histogram);
        assert_eq!(histogram.scale, AxisScale::Log10);
        assert_eq!(result.ranges.len(), histogram.counts.len());
        if histogram.counts.is_empty() {
            assert!(histogram.bin_edges.is_empty());
        } else {
            assert_eq!(histogram.bin_edges.len(), histogram.counts.len() + 1);
            assert!(histogram.bin_edges.iter().all(|edge| edge.is_finite()));
            assert!(histogram
                .bin_edges
                .windows(2)
                .all(|edges| edges[0] < edges[1]));
            assert_eq!(histogram.lo, histogram.bin_edges.first().copied());
            assert_eq!(histogram.hi, histogram.bin_edges.last().copied());
        }
        let mut memberships = vec![0; array.len()];
        let column = Column::new("test", array.clone());
        for (range, &expected) in result.ranges.iter().zip(&histogram.counts) {
            assert!(range.min.is_some() && range.max.is_some());
            let predicate = column
                .prepare_filter(&Filter::Range {
                    min: range.min.clone(),
                    max: range.max.clone(),
                    include_nulls: false,
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
            for (membership, matched) in memberships.iter_mut().zip(matched) {
                *membership += u8::from(matched);
            }
        }
        assert!(
            memberships.iter().all(|&n| n <= 1),
            "native bin ranges overlap"
        );
        assert_eq!(
            histogram.counts.iter().sum::<u64>()
                + histogram.null_count
                + histogram.nan_count
                + histogram.non_positive_count
                + histogram.infinite_count,
            histogram.total
        );
        histogram
    }

    #[test]
    fn geometric_bins_preserve_exact_membership_and_account_for_omissions() {
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(1.0),
            Some(9.0),
            Some(10.0),
            Some(99.0),
            Some(100.0),
            Some(1000.0),
            Some(0.0),
            Some(-1.0),
            Some(f64::NEG_INFINITY),
            Some(f64::INFINITY),
            Some(f64::NAN),
            None,
        ]));
        let histogram = verify(array.clone(), Selection::All(array.len()), 3);
        assert_eq!(histogram.counts, [2, 2, 2]);
        assert_eq!(
            (
                histogram.non_positive_count,
                histogram.infinite_count,
                histogram.nan_count,
                histogram.null_count
            ),
            (2, 2, 1, 1)
        );
        assert_eq!((histogram.lo, histogram.hi), (Some(0.0), Some(3.0)));
        assert_eq!(
            (
                histogram.min_label.as_deref(),
                histogram.max_label.as_deref()
            ),
            (Some("1"), Some("1000"))
        );
        let histogram = verify(array, Selection::Rows(Arc::new(vec![2, 5, 6, 9, 11])), 3);
        assert_eq!(histogram.counts, [0, 1, 1]);
        assert_eq!(
            (
                histogram.non_positive_count,
                histogram.infinite_count,
                histogram.null_count
            ),
            (1, 1, 1)
        );
    }

    #[test]
    fn native_cuts_handle_large_coefficients_and_extreme_float_exponents() {
        let large: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(vec![
                i64::MIN,
                0,
                1,
                9_007_199_254_740_993,
                i64::MAX,
            ])),
            Arc::new(UInt64Array::from(vec![
                u64::MAX - 2,
                u64::MAX - 1,
                u64::MAX,
            ])),
            Arc::new(
                Decimal128Array::from(vec![0, 1, 9_007_199_254_740_993])
                    .with_precision_and_scale(38, 2)
                    .unwrap(),
            ),
            Arc::new(
                Decimal256Array::from(vec![
                    -large,
                    i256::ZERO,
                    i256::ONE,
                    large,
                    large + i256::ONE,
                ])
                .with_precision_and_scale(76, -128)
                .unwrap(),
            ),
            Arc::new(
                Decimal256Array::from(vec![i256::ONE, large])
                    .with_precision_and_scale(76, 76)
                    .unwrap(),
            ),
            Arc::new(Float32Array::from(vec![
                f32::from_bits(1),
                f32::MIN_POSITIVE,
                1.0,
                f32::MAX,
            ])),
            Arc::new(Float64Array::from(vec![
                f64::from_bits(1),
                f64::MIN_POSITIVE,
                1.0,
                f64::MAX,
            ])),
            Arc::new(Float64Array::from(vec![f64::MAX.next_down(), f64::MAX])),
            Arc::new(Float64Array::from(vec![1.0, 1.0_f64.next_up()])),
            Arc::new(Float64Array::from(vec![1.0])),
        ];
        for array in arrays {
            let histogram = verify(array.clone(), Selection::All(array.len()), 40);
            assert!(histogram.lo.unwrap().is_finite());
            assert!(histogram.hi.unwrap().is_finite());
            assert!(histogram.lo < histogram.hi);
            assert!(!histogram.counts.is_empty());
        }
        let values = Float32Array::from(vec![0.0, 0.001, 1.0, 1000.0, f32::INFINITY]);
        let half = arrow::compute::cast(&values, &DataType::Float16).unwrap();
        verify(half, Selection::All(values.len()), 20);

        // Rounding geometric cuts to an integer grid merges bins and can put
        // the final cut exactly at max. Its last singleton still needs width.
        let small: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 3]));
        let histogram = verify(small, Selection::All(3), 40);
        assert_eq!(histogram.counts, [1, 1, 1]);
        assert_eq!(histogram.bin_edges, [
            0.0,
            2.0_f64.log10(),
            3.0_f64.log10(),
            4.0_f64.log10()
        ]);
    }

    #[test]
    fn empty_positive_domains_still_report_exclusions_and_validate_requests() {
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(0.0),
            Some(-1.0),
            Some(f64::INFINITY),
            Some(f64::NAN),
            None,
        ]));
        let histogram = verify(array.clone(), Selection::All(array.len()), 5);
        assert!(histogram.counts.is_empty());
        assert_eq!((histogram.lo, histogram.hi), (None, None));
        assert_eq!(
            (
                histogram.non_positive_count,
                histogram.infinite_count,
                histogram.nan_count,
                histogram.null_count
            ),
            (2, 1, 1, 1)
        );
        let histogram = verify(array.clone(), Selection::Rows(Arc::new(vec![])), 5);
        assert_eq!(histogram.total, 0);
        let context = AnalysisContext::default();
        assert!(build::<Histogram>(&array, &Selection::All(5), 0, &context).is_err());
        let dates: ArrayRef = Arc::new(Date32Array::from(vec![1, 10]));
        assert!(build::<Histogram>(&dates, &Selection::All(2), 5, &context).is_err());
        context.cancel();
        assert!(matches!(
            build::<Histogram>(&array, &Selection::All(5), 5, &context),
            Err(Error::Cancelled)
        ));
    }
}
