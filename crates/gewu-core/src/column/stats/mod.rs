//! Shared native binning for statistics and filter charts. Approximate axis
//! values are display-only; counting and selecting a bin use the same cuts.

use arrow::{
    array::{
        downcast_integer_array, downcast_temporal_array, Array, ArrayRef, AsArray, PrimitiveArray,
    },
    datatypes::*,
};

use crate::{analysis::AnalysisContext, scope::Selection, Bound, Error, Result};

mod logarithmic;
mod scale;

pub use scale::AxisScale;

/// Distribution of a numeric column for statistics and filter charts.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    /// Binning coordinate space. Log10 axes contain log10(value); labels and
    /// range bounds always retain the original values.
    pub scale: AxisScale,
    /// Approximate display axis over the whole source column, not just the
    /// counted rows, so it stays put while filters change. Native values decide
    /// bin membership; use [`RangeHistogram::ranges`] for exact filter bounds.
    /// Linear bins include infinities at the ends. `None` when there is no
    /// eligible finite domain (logarithmic bins require positive values).
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// Exact finite source extrema for display, independent of plotting coordinates.
    /// Absent when the source has no finite domain.
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    /// Linear integer grid (including dates and times). The axis encloses
    /// `min..max + 1`, and small spans have exactly one integer per bin.
    pub integer: bool,
    pub counts: Vec<u64>,
    /// Log10 display edges for the retained native bins, with `counts.len() + 1`
    /// strictly increasing values. Empty for linear or empty histograms. Native
    /// cuts that collapse at f64 log precision are nudged apart for display only.
    pub bin_edges: Vec<f64>,
    pub null_count: u64,
    pub nan_count: u64,
    /// Finite non-positive values omitted by logarithmic binning; zero for linear bins.
    pub non_positive_count: u64,
    /// Infinities omitted by logarithmic binning. Linear bins include them at the edges.
    pub infinite_count: u64,
    /// Rows counted (including nulls and NaN).
    pub total: u64,
}

/// Drawing coordinates and exact filter endpoints for the same bins.
/// Coordinates can be approximate; submit `ranges` to filter by a bin.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeHistogram {
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeBounds {
    pub min: Option<crate::Bound>,
    pub max: Option<crate::Bound>,
}
/// Range-filter charts have a bounded number of options, independent of row count.
const MAX_BINS: u32 = 60;

/// A missing finite domain is different from an unsupported physical type.
enum Domain<H> {
    Bins(H),
    Empty,
    InfiniteOnly,
    Unsupported,
}

/// Counting and native cuts are shared; only callers needing selectable bins
/// materialize typed bounds. The closure is never evaluated for statistics.
trait Output: Sized {
    fn finish(
        histogram: Histogram,
        ranges: impl FnOnce() -> Result<Vec<RangeBounds>>,
    ) -> Result<Self>;
}

impl Output for Histogram {
    fn finish(histogram: Histogram, _: impl FnOnce() -> Result<Vec<RangeBounds>>) -> Result<Self> {
        Ok(histogram)
    }
}

impl Output for RangeHistogram {
    fn finish(
        histogram: Histogram,
        ranges: impl FnOnce() -> Result<Vec<RangeBounds>>,
    ) -> Result<Self> {
        Ok(Self {
            histogram,
            ranges: ranges()?,
        })
    }
}

pub(crate) fn build(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
) -> Result<Option<RangeHistogram>> {
    Ok(
        match build_bins(array, rows, bins.min(MAX_BINS), &AnalysisContext::default())? {
            Domain::Bins(histogram) => Some(histogram),
            Domain::Empty | Domain::InfiniteOnly | Domain::Unsupported => None,
        },
    )
}

pub(crate) fn build_scaled(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    scale: AxisScale,
) -> Result<Option<RangeHistogram>> {
    match scale {
        AxisScale::Linear => build(array, rows, bins),
        AxisScale::Log10 => {
            logarithmic::build(array, rows, bins.min(MAX_BINS), &AnalysisContext::default())
                .map(Some)
        }
    }
}

/// Statistics use the requested bin count and the caller's row scope, while
/// sharing exact cuts with filter charts. Empty/all-NaN columns have no axis;
/// an infinity-only column has no finite domain and is an explicit error.
pub(crate) fn counts(array: &ArrayRef, rows: &Selection, bins: u32) -> Result<Histogram> {
    counts_with_context(array, rows, bins, &AnalysisContext::default())
}

pub(crate) fn counts_with_context(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    context: &AnalysisContext,
) -> Result<Histogram> {
    statistics(array, rows, bins, context)
}

pub(crate) fn counts_scaled_with_context(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    scale: AxisScale,
    context: &AnalysisContext,
) -> Result<Histogram> {
    match scale {
        AxisScale::Linear => counts_with_context(array, rows, bins, context),
        AxisScale::Log10 => logarithmic::build(array, rows, bins, context),
    }
}

pub(crate) fn ranges(array: &ArrayRef, rows: &Selection, bins: u32) -> Result<RangeHistogram> {
    statistics(array, rows, bins, &AnalysisContext::default())
}

fn statistics<H: Output>(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    context: &AnalysisContext,
) -> Result<H> {
    context.check()?;
    let logical = crate::LogicalType::from_arrow(array.data_type());
    let is_dict = matches!(array.data_type(), DataType::Dictionary(..));
    if !logical.capabilities(is_dict).histogram {
        return Err(Error::InvalidArgument(format!(
            "{} does not support histograms",
            array.data_type()
        )));
    }
    match build_bins(array, rows, bins, context)? {
        Domain::Bins(histogram) => return Ok(histogram),
        Domain::Empty => {}
        Domain::InfiniteOnly => {
            return Err(Error::InvalidArgument(
                "histogram needs at least one finite value".into(),
            ));
        }
        Domain::Unsupported => {
            return Err(Error::InvalidArgument(
                "column has no representable histogram domain".into(),
            ));
        }
    }
    let total = rows.len() as u64;
    let null_count = match rows {
        Selection::All(_) => array.null_count() as u64,
        Selection::Rows(_) if array.null_count() == 0 => 0,
        Selection::Rows(_) => {
            let mut null_count = 0;
            for (index, row) in rows.iter().enumerate() {
                context.check_at(index)?;
                null_count += u64::from(array.is_null(row));
            }
            null_count
        }
    };
    context.check()?;
    H::finish(
        Histogram {
            scale: AxisScale::Linear,
            lo: None,
            hi: None,
            min_label: None,
            max_label: None,
            integer: array.data_type().is_integer() || array.data_type().is_temporal(),
            counts: Vec::new(),
            bin_edges: Vec::new(),
            null_count,
            nan_count: total - null_count,
            non_positive_count: 0,
            infinite_count: 0,
            total,
        },
        || Ok(Vec::new()),
    )
}

fn build_bins<H: Output>(
    array: &ArrayRef,
    rows: &Selection,
    bins: u32,
    context: &AnalysisContext,
) -> Result<Domain<H>> {
    context.check()?;
    if bins == 0 {
        return Err(Error::InvalidArgument("bins must be at least 1".into()));
    }
    let bins = bins as usize;
    match array.data_type() {
        data_type if data_type.is_integer() => {
            downcast_integer_array!(array => integer(array, rows, bins, context), _ => unreachable!("integer dispatch"))
        }
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(..) => {
            downcast_temporal_array!(array => integer(array, rows, bins, context), _ => Ok(Domain::Unsupported))
        }
        DataType::Duration(TimeUnit::Second) => integer(
            array.as_primitive::<DurationSecondType>(),
            rows,
            bins,
            context,
        ),
        DataType::Duration(TimeUnit::Millisecond) => integer(
            array.as_primitive::<DurationMillisecondType>(),
            rows,
            bins,
            context,
        ),
        DataType::Duration(TimeUnit::Microsecond) => integer(
            array.as_primitive::<DurationMicrosecondType>(),
            rows,
            bins,
            context,
        ),
        DataType::Duration(TimeUnit::Nanosecond) => integer(
            array.as_primitive::<DurationNanosecondType>(),
            rows,
            bins,
            context,
        ),
        DataType::Float16 => floating(array.as_primitive::<Float16Type>(), rows, bins, context),
        DataType::Float32 => floating(array.as_primitive::<Float32Type>(), rows, bins, context),
        DataType::Float64 => floating(array.as_primitive::<Float64Type>(), rows, bins, context),
        DataType::Decimal32(_, scale) => decimal(
            array.as_primitive::<Decimal32Type>(),
            rows,
            bins,
            context,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i32,
        ),
        DataType::Decimal64(_, scale) => decimal(
            array.as_primitive::<Decimal64Type>(),
            rows,
            bins,
            context,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i64,
        ),
        DataType::Decimal128(_, scale) => decimal(
            array.as_primitive::<Decimal128Type>(),
            rows,
            bins,
            context,
            *scale,
            i256::from_i128,
            |value| value.to_i128().expect("native coefficient domain"),
        ),
        DataType::Decimal256(_, scale) => decimal(
            array.as_primitive::<Decimal256Type>(),
            rows,
            bins,
            context,
            *scale,
            |value| value,
            |value| value,
        ),
        // Dictionaries still support typed text ranges, but have no chart until
        // their encoded values can be counted without decoding the source column.
        _ => Ok(Domain::Unsupported),
    }
}

fn extrema<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    context: &AnalysisContext,
) -> Result<Option<(T::Native, T::Native)>>
where
    T::Native: Ord,
{
    let mut extrema: Option<(T::Native, T::Native)> = None;
    for (index, value) in array.iter().enumerate() {
        context.check_at(index)?;
        if let Some(value) = value {
            extrema = Some(match extrema {
                Some((lo, hi)) => (lo.min(value), hi.max(value)),
                None => (value, value),
            });
        }
    }
    context.check()?;
    Ok(extrema)
}

fn integer<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
) -> Result<Domain<H>>
where
    T::Native: Into<i128> + Ord,
{
    let Some((min, max)) = extrema(array, context)? else {
        return Ok(Domain::Empty);
    };
    let (min, max) = (min.into(), max.into());
    // Every supported integer/temporal array fits i64 or u64, so this inclusive
    // domain and the small interpolation products fit comfortably in i128.
    let span = max - min + 1;
    let n = span.min(bins as i128) as usize;
    let cuts = (1..n)
        .map(|i| {
            context.check_at(i)?;
            Ok(min + (span * i as i128 + n as i128 - 1) / n as i128)
        })
        .collect::<Result<Vec<_>>>()?;
    let bound = |value: i128| {
        if array.data_type().is_unsigned_integer() {
            Bound::from_u64(array.data_type(), value as u64)
        } else {
            Bound::from_i64(array.data_type(), value as i64)
        }
    };
    let mut lo = min as f64;
    let mut hi = (max + 1) as f64;
    if lo as i128 > min {
        lo = lo.next_down();
    }
    if (hi as i128) < max + 1 {
        hi = hi.next_up();
    }
    let mut histogram = count(array, rows, n, true, axis(lo, hi, n), context, |value| {
        Some(cuts.partition_point(|&cut| cut <= value.into()))
    })?;
    histogram.min_label = Some(bound(min)?.display().to_owned());
    histogram.max_label = Some(bound(max)?.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, context, bound, |value| bound(value - 1))
    })
    .map(Domain::Bins)
}

fn decimal<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
    scale: i8,
    widen: impl Fn(T::Native) -> i256,
    narrow: impl Fn(i256) -> T::Native,
) -> Result<Domain<H>>
where
    T::Native: Ord,
{
    let Some((min, max)) = extrema(array, context)? else {
        return Ok(Domain::Empty);
    };
    let (min, max) = (widen(min), widen(max));
    // Malformed arrays can hold coefficients outside their declared decimal
    // domain. If their span exceeds i256, omit the chart instead of rounding it.
    let Some(span) = max
        .checked_sub(min)
        .and_then(|span| span.checked_add(i256::ONE))
    else {
        return Ok(Domain::Unsupported);
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
            context.check_at(i)?;
            // ceil(span * i / n), without overflowing the wide coefficient type.
            Ok(min
                + quotient * i256::from_i128(i as i128)
                + i256::from_i128((remainder * i as i128 + n as i128 - 1) / n as i128))
        })
        .collect::<Result<Vec<_>>>()?;
    let bound = |coefficient| Bound::from_decimal(array.data_type(), coefficient, scale);
    let (lower, upper) = (bound(min)?, bound(max)?);
    let (lo, hi) = (lower.axis_value(), upper.axis_value());
    let native_cuts = cuts
        .iter()
        .copied()
        .enumerate()
        .map(|(index, value)| {
            context.check_at(index)?;
            Ok(narrow(value))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut histogram = count(array, rows, n, false, axis(lo, hi, n), context, |value| {
        Some(native_cuts.partition_point(|&cut| cut <= value))
    })?;
    histogram.min_label = Some(lower.display().to_owned());
    histogram.max_label = Some(upper.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, context, bound, |value| bound(value - i256::ONE))
    })
    .map(Domain::Bins)
}

fn floating<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    bins: usize,
    context: &AnalysisContext,
) -> Result<Domain<H>>
where
    T::Native: Into<f64>,
{
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut has_infinite = false;
    for (index, value) in array.iter().enumerate() {
        context.check_at(index)?;
        let Some(value) = value else { continue };
        let value: f64 = value.into();
        has_infinite |= value.is_infinite();
        if value.is_finite() {
            min = min.min(value);
            max = max.max(value);
        }
    }
    context.check()?;
    if min > max {
        return Ok(if has_infinite {
            Domain::InfiniteOnly
        } else {
            Domain::Empty
        });
    }
    let mut cuts = Vec::with_capacity(bins - 1);
    if min < max {
        let span = max - min;
        for i in 1..bins {
            context.check_at(i)?;
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
    let n = cuts.len() + 1;
    let mut histogram = count(array, rows, n, false, axis(min, max, n), context, |value| {
        let value: f64 = value.into();
        (!value.is_nan()).then(|| cuts.partition_point(|&cut| cut <= value))
    })?;
    histogram.min_label = Some(bound(min)?.display().to_owned());
    histogram.max_label = Some(bound(max)?.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, context, bound, |value: f64| bound(value.next_down()))
    })
    .map(Domain::Bins)
}

fn bin_ranges<C: Copy>(
    cuts: &[C],
    context: &AnalysisContext,
    lower: impl Fn(C) -> Result<Bound>,
    upper: impl Fn(C) -> Result<Bound>,
) -> Result<Vec<RangeBounds>> {
    (0..=cuts.len())
        .map(|i| {
            context.check_at(i)?;
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
    if lo.is_finite() && hi.is_finite() && lo < hi && (hi - lo).is_finite() {
        return (lo, hi);
    }
    let expanded = (lo.next_down(), hi.next_up());
    if expanded.0.is_finite()
        && expanded.1.is_finite()
        && expanded.0 < expanded.1
        && (expanded.1 - expanded.0).is_finite()
    {
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
    context: &AnalysisContext,
    classify: impl Fn(T::Native) -> Option<usize>,
) -> Result<Histogram> {
    context.check()?;
    let mut histogram = Histogram {
        scale: AxisScale::Linear,
        lo: Some(axis.0),
        hi: Some(axis.1),
        min_label: None,
        max_label: None,
        integer,
        counts: vec![0; bins],
        bin_edges: Vec::new(),
        null_count: 0,
        nan_count: 0,
        non_positive_count: 0,
        infinite_count: 0,
        total: rows.len() as u64,
    };
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if array.is_null(row) {
            histogram.null_count += 1;
        } else if let Some(bin) = classify(array.value(row)) {
            histogram.counts[bin] += 1;
        } else {
            histogram.nan_count += 1;
        }
    }
    context.check()?;
    Ok(histogram)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, sync::Arc};

    use arrow::array::{
        Date32Array, Decimal128Array, Decimal256Array, DictionaryArray, Float32Array, Float64Array,
        Int64Array, Int8Array, Time32MillisecondArray, TimestampNanosecondArray, UInt64Array,
    };

    use super::*;
    use crate::{column::Column, engine::CancelToken, Filter};

    #[test]
    fn cancellation_interrupts_histogram_counting() {
        let array = Int64Array::from_iter_values(0..8193);
        let rows = Selection::All(array.len());
        let context = AnalysisContext::default();
        let visited = Cell::new(0);
        let result = count(&array, &rows, 1, true, (0.0, 8193.0), &context, |_| {
            visited.set(visited.get() + 1);
            context.cancel();
            Some(0)
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(
            visited.get() < array.len(),
            "cancellation must stop the scan"
        );

        let array: ArrayRef = Arc::new(array);
        assert!(matches!(
            counts_with_context(&array, &rows, 1, &context),
            Err(Error::Cancelled)
        ));
        assert_eq!(counts(&array, &rows, 1).unwrap().counts, [8193]);
    }

    fn verify_bins(array: ArrayRef, rows: Selection, bins: u32) -> RangeHistogram {
        let result = build(&array, &rows, bins).unwrap().unwrap();
        let statistics = counts(&array, &rows, bins.min(MAX_BINS)).unwrap();
        assert_eq!(statistics, result.histogram);
        assert_eq!(result.ranges.len(), result.histogram.counts.len());
        assert!(result.ranges.first().unwrap().min.is_none());
        assert!(result.ranges.last().unwrap().max.is_none());
        let mut selected = vec![0_u32; array.len()];
        let column = Column::new("test", array.clone(), true);
        for (bounds, &expected) in result.ranges.iter().zip(&result.histogram.counts) {
            let predicate = column
                .prepare_filter(&Filter::Range {
                    include_nulls: false,
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
    fn statistics_preserve_requested_bins_and_exact_decimal_membership() {
        let base = 9_007_199_254_740_992_i128;
        let array: ArrayRef = Arc::new(
            Decimal128Array::from_iter_values(base..base + 100)
                .with_precision_and_scale(38, 2)
                .unwrap(),
        );
        let histogram = counts(&array, &Selection::All(100), 100).unwrap();
        assert_eq!(histogram.counts, vec![1; 100]);
        let selected = Selection::Rows(Arc::new(vec![1, 99]));
        let histogram = counts(&array, &selected, 100).unwrap();
        assert_eq!(histogram.total, 2);
        assert_eq!(histogram.counts[0], 0);
        assert_eq!(histogram.counts[1], 1);
        assert_eq!(histogram.counts[99], 1);
        assert_eq!(histogram.counts.iter().sum::<u64>(), 2);
        let distribution = ranges(&array, &selected, 100).unwrap();
        assert_eq!(distribution.histogram, histogram);
        assert_eq!(distribution.ranges.len(), 100);
    }

    #[test]
    fn duration_units_support_statistics_and_exact_ranges() {
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            let array = arrow::compute::cast(
                &Int64Array::from(vec![Some(-1), Some(0), Some(1), None]),
                &DataType::Duration(unit),
            )
            .unwrap();
            let result = verify_bins(array, Selection::All(4), 3);
            assert_eq!(result.histogram.counts, [1, 1, 1]);
            assert_eq!(result.histogram.null_count, 1);
        }
    }

    #[test]
    fn missing_finite_domain_preserves_nan_null_and_infinity_semantics() {
        for values in [vec![], vec![None], vec![Some(f64::NAN), None]] {
            let array: ArrayRef = Arc::new(Float64Array::from(values));
            let rows = Selection::All(array.len());
            let h = counts(&array, &rows, 3).unwrap();
            assert!(h.counts.is_empty());
            assert_eq!((h.lo, h.hi), (None, None));
            assert_eq!(h.total, array.len() as u64);
            assert_eq!(h.null_count, array.null_count() as u64);
            assert_eq!(h.null_count + h.nan_count, h.total);
            assert!(build(&array, &rows, 3).unwrap().is_none());
        }
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(f64::NAN),
            None,
            Some(f64::NAN),
        ]));
        for (rows, total, nulls, nans) in [
            (Selection::All(3), 3, 1, 2),
            (Selection::Rows(Arc::new(vec![1, 2])), 2, 1, 1),
            (Selection::Rows(Arc::new(vec![])), 0, 0, 0),
        ] {
            let h = counts(&array, &rows, 40).unwrap();
            assert_eq!((h.total, h.null_count, h.nan_count), (total, nulls, nans));
            assert!(h.counts.is_empty());
            assert!(h.min_label.is_none() && h.max_label.is_none());
            assert_eq!(ranges(&array, &rows, 40).unwrap().histogram, h);
        }
        for value in [f64::INFINITY, f64::NEG_INFINITY] {
            let array: ArrayRef =
                Arc::new(Float64Array::from(vec![Some(f64::NAN), None, Some(value)]));
            assert!(matches!(
                counts(&array, &Selection::All(3), 40),
                Err(Error::InvalidArgument(_))
            ));
            assert!(ranges(&array, &Selection::All(3), 40).is_err());
            assert!(build(&array, &Selection::All(3), 40).unwrap().is_none());
        }
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
        assert_eq!(
            result.histogram.min_label.as_deref(),
            Some("9007199254740992")
        );
        assert_eq!(
            result.histogram.max_label.as_deref(),
            Some("9007199254740993")
        );
    }

    #[test]
    fn decimal_bins_preserve_large_coefficients_and_negative_scales() {
        let tenths: ArrayRef = Arc::new(
            Decimal128Array::from_iter_values(0..10)
                .with_precision_and_scale(10, 1)
                .unwrap(),
        );
        let result = verify_bins(tenths, Selection::All(10), 3);
        assert_eq!(result.histogram.counts, [4, 3, 3]);
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
        let infinities: ArrayRef = Arc::new(Float64Array::from(vec![
            f64::NEG_INFINITY,
            0.0,
            1.0,
            2.0,
            f64::INFINITY,
        ]));
        let result = verify_bins(infinities, Selection::All(5), 2);
        assert_eq!(
            (result.histogram.lo, result.histogram.hi),
            (Some(0.0), Some(2.0))
        );
        assert_eq!(result.histogram.counts, [2, 3]);
        for (values, bins) in [
            (vec![-f64::MAX, 0.0, f64::MAX], 3),
            (vec![1.0, 1.0_f64.next_up()], 60),
            (vec![0.0, 0.0_f64.next_up()], 60),
            (vec![f64::MAX], 60),
        ] {
            let len = values.len();
            let single = values.len() == 1;
            let result = verify_bins(
                Arc::new(Float64Array::from(values)),
                Selection::All(len),
                bins,
            );
            assert_eq!(result.histogram.counts, vec![1; len]);
            assert!((result.histogram.hi.unwrap() - result.histogram.lo.unwrap()).is_finite());
            if single {
                assert_eq!(result.histogram.min_label, Some(format!("{:.0}", f64::MAX)));
                assert_eq!(result.histogram.max_label, Some(format!("{:.0}", f64::MAX)));
            }
        }
    }

    #[test]
    fn omitted_charts_and_empty_facets_are_explicit() {
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(vec![None])),
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
