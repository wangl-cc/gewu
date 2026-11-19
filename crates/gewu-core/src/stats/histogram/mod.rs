//! Shared native binning for statistics and filter charts. Approximate axis
//! values are display-only; counting and selecting a bin use the same cuts.

mod logarithmic;

use arrow::{
    array::{
        downcast_integer_array, downcast_temporal_array, Array, ArrayRef, AsArray, PrimitiveArray,
    },
    datatypes::*,
};

use super::AxisScale;
use crate::{
    col_view::{ColView, Selection},
    scalar::Bound,
    stats::{scan::for_each_valid, Result},
};

/// Distribution of a numeric column for statistics and filter charts.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    /// Binning coordinate space. Log10 axes contain log10(value); labels and
    /// range bounds always retain the original values.
    pub scale: AxisScale,
    /// Approximate display axis over this View's selected rows. Native values
    /// decide bin membership; use [`RangeHistogram::ranges`] for exact filter bounds.
    /// Linear bins include infinities at the ends. `None` when there is no
    /// eligible finite domain (logarithmic bins require positive values).
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// Exact finite selected-value extrema for display, independent of plotting coordinates.
    /// Absent when the selected rows have no finite domain.
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
    /// Finite non-positive values omitted by logarithmic binning; zero for linear bins.
    pub non_positive_count: u64,
    /// Infinities omitted by logarithmic binning. Linear bins include them at the edges.
    pub infinite_count: u64,
}

/// Drawing coordinates and exact filter endpoints for the same bins.
/// Coordinates can be approximate; submit `ranges` to filter by a bin.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeHistogram {
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
}

/// Display and filter input projections of an exact native endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeBound {
    pub value: String,
    pub label: String,
    pub coordinate: f64,
}

impl From<Bound> for RangeBound {
    fn from(bound: Bound) -> Self {
        Self {
            value: bound.filter_text(),
            label: bound.display().to_owned(),
            coordinate: bound.axis_value(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeBounds {
    pub min: Option<RangeBound>,
    pub max: Option<RangeBound>,
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

pub(crate) fn build_scaled(
    view: ColView<'_>,
    bins: u32,
    scale: AxisScale,
) -> Result<Option<RangeHistogram>> {
    match scale {
        AxisScale::Linear => Ok(
            match build_bins(view.array(), &view.selection(), bins.min(MAX_BINS))? {
                Domain::Bins(histogram) => Some(histogram),
                Domain::Empty | Domain::InfiniteOnly | Domain::Unsupported => None,
            },
        ),
        AxisScale::Log10 => logarithmic::build(view, bins.min(MAX_BINS)).map(Some),
    }
}

pub(crate) fn counts(view: ColView<'_>, bins: u32) -> Result<Histogram> {
    if !supports_histogram(view.data_type()) {
        return Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Distribution,
            data_type: view.data_type().clone(),
        });
    }
    statistics(view.array(), &view.selection(), bins)
}

pub(crate) fn counts_scaled(view: ColView<'_>, bins: u32, scale: AxisScale) -> Result<Histogram> {
    match scale {
        AxisScale::Linear => counts(view, bins),
        AxisScale::Log10 => logarithmic::build(view, bins),
    }
}

/// Storage types handled by native histogram binning.
pub(crate) fn supports_histogram(data_type: &DataType) -> bool {
    use DataType::*;
    matches!(
        data_type,
        Int8 | Int16
            | Int32
            | Int64
            | UInt8
            | UInt16
            | UInt32
            | UInt64
            | Float16
            | Float32
            | Float64
            | Decimal32(..)
            | Decimal64(..)
            | Decimal128(..)
            | Decimal256(..)
            | Date32
            | Date64
            | Time32(_)
            | Time64(_)
            | Timestamp(..)
            | Duration(_)
    )
}

fn statistics(array: &ArrayRef, selection: &Selection<'_>, bins: u32) -> Result<Histogram> {
    let histogram = supports_histogram(array.data_type());
    if !histogram {
        return Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Distribution,
            data_type: array.data_type().clone(),
        });
    }
    match build_bins(array, selection, bins)? {
        Domain::Bins(histogram) => return Ok(histogram),
        Domain::Empty => {}
        Domain::InfiniteOnly => {
            return Err(crate::AnalysisError::NoFiniteValues);
        }
        Domain::Unsupported => {
            return Err(crate::AnalysisError::UnrepresentableDomain {
                data_type: array.data_type().clone(),
            });
        }
    }
    Ok(Histogram {
        scale: AxisScale::Linear,
        lo: None,
        hi: None,
        min_label: None,
        max_label: None,
        integer: array.data_type().is_integer() || array.data_type().is_temporal(),
        counts: Vec::new(),
        bin_edges: Vec::new(),
        non_positive_count: 0,
        infinite_count: 0,
    })
}

fn build_bins<H: Output>(
    array: &ArrayRef,
    selection: &Selection<'_>,
    bins: u32,
) -> Result<Domain<H>> {
    if bins == 0 {
        return Err(crate::AnalysisError::InvalidBinCount {
            operation: crate::AnalysisOperation::Distribution,
            value: bins,
            min: 1,
            max: u32::MAX,
        });
    }
    let bins = bins as usize;
    match array.data_type() {
        data_type if data_type.is_integer() => {
            downcast_integer_array!(array => integer(array, selection, bins), _ => unreachable!("integer dispatch"))
        }
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(..) => {
            downcast_temporal_array!(array => integer(array, selection, bins), _ => Ok(Domain::Unsupported))
        }
        DataType::Duration(TimeUnit::Second) => {
            integer(array.as_primitive::<DurationSecondType>(), selection, bins)
        }
        DataType::Duration(TimeUnit::Millisecond) => integer(
            array.as_primitive::<DurationMillisecondType>(),
            selection,
            bins,
        ),
        DataType::Duration(TimeUnit::Microsecond) => integer(
            array.as_primitive::<DurationMicrosecondType>(),
            selection,
            bins,
        ),
        DataType::Duration(TimeUnit::Nanosecond) => integer(
            array.as_primitive::<DurationNanosecondType>(),
            selection,
            bins,
        ),
        DataType::Float16 => floating(array.as_primitive::<Float16Type>(), selection, bins),
        DataType::Float32 => floating(array.as_primitive::<Float32Type>(), selection, bins),
        DataType::Float64 => floating(array.as_primitive::<Float64Type>(), selection, bins),
        DataType::Decimal32(_, scale) => decimal(
            array.as_primitive::<Decimal32Type>(),
            selection,
            bins,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i32,
        ),
        DataType::Decimal64(_, scale) => decimal(
            array.as_primitive::<Decimal64Type>(),
            selection,
            bins,
            *scale,
            |value| i256::from_i128(i128::from(value)),
            |value| value.to_i128().expect("native coefficient domain") as i64,
        ),
        DataType::Decimal128(_, scale) => decimal(
            array.as_primitive::<Decimal128Type>(),
            selection,
            bins,
            *scale,
            i256::from_i128,
            |value| value.to_i128().expect("native coefficient domain"),
        ),
        DataType::Decimal256(_, scale) => decimal(
            array.as_primitive::<Decimal256Type>(),
            selection,
            bins,
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
    selection: &Selection<'_>,
) -> Option<(T::Native, T::Native)>
where
    T::Native: Ord,
{
    let mut extrema: Option<(T::Native, T::Native)> = None;
    for_each_valid(array, selection, |_, value| {
        extrema = Some(match extrema {
            Some((lo, hi)) => (lo.min(value), hi.max(value)),
            None => (value, value),
        });
    });
    extrema
}

fn integer<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
) -> Result<Domain<H>>
where
    T::Native: Into<i128> + Ord,
{
    let Some((min, max)) = extrema(array, selection) else {
        return Ok(Domain::Empty);
    };
    let (min, max) = (min.into(), max.into());
    // Every supported integer/temporal array fits i64 or u64, so this inclusive
    // domain and the small interpolation products fit comfortably in i128.
    let span = max - min + 1;
    let n = span.min(bins as i128) as usize;
    let cuts: Vec<_> = (1..n)
        .map(|i| min + (span * i as i128 + n as i128 - 1) / n as i128)
        .collect();
    let bound = |value: i128| {
        if array.data_type().is_unsigned_integer() {
            Ok(Bound::from_u64(array.data_type(), value as u64)?)
        } else {
            Ok(Bound::from_i64(array.data_type(), value as i64)?)
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
    let per_unit = n as f64 / span as f64;
    let mut histogram = count(array, selection, n, true, axis(lo, hi, n), |value| {
        let value: i128 = value.into();
        Some(locate(
            &cuts,
            value,
            ((value - min) as f64 * per_unit) as usize,
        ))
    });
    histogram.min_label = Some(bound(min)?.display().to_owned());
    histogram.max_label = Some(bound(max)?.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, bound, |value| bound(value - 1))
    })
    .map(Domain::Bins)
}

fn decimal<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
    scale: i8,
    widen: impl Fn(T::Native) -> i256,
    narrow: impl Fn(i256) -> T::Native,
) -> Result<Domain<H>>
where
    T::Native: Ord,
{
    let Some((min, max)) = extrema(array, selection) else {
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
    let cuts: Vec<_> = (1..n)
        .map(|i| {
            // ceil(span * i / n), without overflowing the wide coefficient type.
            min + quotient * i256::from_i128(i as i128)
                + i256::from_i128((remainder * i as i128 + n as i128 - 1) / n as i128)
        })
        .collect();
    let bound = |coefficient| Ok(Bound::from_decimal(array.data_type(), coefficient, scale)?);
    let (lower, upper) = (bound(min)?, bound(max)?);
    let (lo, hi) = (lower.axis_value(), upper.axis_value());
    let native_cuts: Vec<_> = cuts.iter().copied().map(narrow).collect();
    let mut histogram = count(array, selection, n, false, axis(lo, hi, n), |value| {
        Some(native_cuts.partition_point(|&cut| cut <= value))
    });
    histogram.min_label = Some(lower.display().to_owned());
    histogram.max_label = Some(upper.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, bound, |value| bound(value - i256::ONE))
    })
    .map(Domain::Bins)
}

fn floating<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
) -> Result<Domain<H>>
where
    T::Native: Into<f64>,
{
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut has_infinite = false;
    for_each_valid(array, selection, |_, value| {
        let value: f64 = value.into();
        has_infinite |= value.is_infinite();
        if value.is_finite() {
            min = min.min(value);
            max = max.max(value);
        }
    });
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
    let bound = |value| Ok(Bound::from_f64(array.data_type(), value)?);
    let n = cuts.len() + 1;
    let per_unit = Some(bins as f64 / (max - min))
        .filter(|per| per.is_finite())
        .unwrap_or(0.0);
    let mut histogram = count(array, selection, n, false, axis(min, max, n), |value| {
        let value: f64 = value.into();
        (!value.is_nan()).then(|| locate(&cuts, value, ((value - min) * per_unit) as usize))
    });
    histogram.min_label = Some(bound(min)?.display().to_owned());
    histogram.max_label = Some(bound(max)?.display().to_owned());
    H::finish(histogram, || {
        bin_ranges(&cuts, bound, |value: f64| bound(value.next_down()))
    })
    .map(Domain::Bins)
}

/// `cuts.partition_point(|&cut| cut <= value)` for ascending `cuts`, walking
/// from an interpolated `guess`. Exact for any guess; near-uniform cuts make
/// it O(1) instead of a branchy binary search per value.
#[inline]
fn locate<C: PartialOrd + Copy>(cuts: &[C], value: C, guess: usize) -> usize {
    let mut index = guess.min(cuts.len());
    while index > 0 && cuts[index - 1] > value {
        index -= 1;
    }
    while index < cuts.len() && cuts[index] <= value {
        index += 1;
    }
    index
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
                    Some(lower(cuts[i - 1])?.into())
                },
                max: cuts
                    .get(i)
                    .copied()
                    .map(&upper)
                    .transpose()?
                    .map(Into::into),
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
    selection: &Selection<'_>,
    bins: usize,
    integer: bool,
    axis: (f64, f64),
    classify: impl Fn(T::Native) -> Option<usize>,
) -> Histogram {
    let mut histogram = Histogram {
        scale: AxisScale::Linear,
        lo: Some(axis.0),
        hi: Some(axis.1),
        min_label: None,
        max_label: None,
        integer,
        counts: vec![0; bins],
        bin_edges: Vec::new(),
        non_positive_count: 0,
        infinite_count: 0,
    };
    for_each_valid(array, selection, |_, value| {
        if let Some(bin) = classify(value) {
            histogram.counts[bin] += 1;
        }
    });
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
    use crate::{col_view::test::Fixture, Filter};

    fn verify_bins(fixture: Fixture, bins: u32) -> RangeHistogram {
        let view = fixture.view();
        let result = build_scaled(view, bins, AxisScale::Linear)
            .unwrap()
            .unwrap();
        let statistics = counts(view, bins.min(MAX_BINS)).unwrap();
        assert_eq!(statistics, result.histogram);
        assert_eq!(result.ranges.len(), result.histogram.counts.len());
        assert!(result.ranges.first().unwrap().min.is_none());
        assert!(result.ranges.last().unwrap().max.is_none());
        let mut selected = vec![0_u32; view.array().len()];
        for (bounds, &expected) in result.ranges.iter().zip(&result.histogram.counts) {
            let predicate = view
                .column()
                .compile_filter(&Filter::range(
                    (bounds.min.clone()).map(|b| b.value),
                    (bounds.max.clone()).map(|b| b.value),
                    false,
                ))
                .unwrap();
            let mut matched = vec![false; view.array().len()];
            predicate.evaluate(0..view.array().len(), &mut matched);
            assert_eq!(
                view.rows().iter().filter(|&row| matched[row]).count() as u64,
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
        assert_eq!(
            result.histogram.counts.iter().sum::<u64>()
                + crate::stats::count_nulls(view)
                + crate::stats::analyze_statistics(view).unwrap().nan_count,
            view.len() as u64
        );
        result
    }

    #[test]
    fn locate_matches_partition_point_for_any_guess() {
        let cuts = [-3.0, -1.0, 0.0, 0.5, 4.0, f64::MAX];
        for value in [
            f64::NEG_INFINITY,
            -5.0,
            -3.0,
            -2.0,
            0.0,
            0.25,
            4.0,
            9.0,
            f64::INFINITY,
        ] {
            let expected = cuts.partition_point(|&cut| cut <= value);
            for guess in [0, 1, 3, 5, 6, 7, usize::MAX] {
                assert_eq!(
                    locate(&cuts, value, guess),
                    expected,
                    "{value} from {guess}"
                );
            }
        }
    }

    #[test]
    fn statistics_preserve_requested_bins_and_exact_decimal_membership() {
        let base = 9_007_199_254_740_992_i128;
        let array: ArrayRef = Arc::new(
            Decimal128Array::from_iter_values(base..base + 100)
                .with_precision_and_scale(38, 2)
                .unwrap(),
        );
        let fixture = crate::col_view::test::Fixture::all(&array);
        let histogram = counts(fixture.view(), 100).unwrap();
        assert_eq!(histogram.counts, vec![1; 100]);
        let selected = Fixture::new(&array, vec![1, 99]);
        let histogram = counts(selected.view(), 100).unwrap();
        assert_eq!(histogram.counts.len(), 99);
        assert_eq!(histogram.counts[0], 1);
        assert_eq!(histogram.counts[98], 1);
        assert_eq!(histogram.counts.iter().sum::<u64>(), 2);
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
            let result = verify_bins(Fixture::all(&array), 3);
            assert_eq!(result.histogram.counts, [1, 1, 1]);
        }
    }

    #[test]
    fn missing_finite_domain_preserves_nan_null_and_infinity_semantics() {
        for values in [vec![], vec![None], vec![Some(f64::NAN), None]] {
            let array: ArrayRef = Arc::new(Float64Array::from(values));
            let all = Fixture::all(&array);
            let h = counts(all.view(), 3).unwrap();
            assert!(h.counts.is_empty());
            assert_eq!((h.lo, h.hi), (None, None));
            assert_eq!(
                crate::stats::count_nulls(all.view()),
                array.null_count() as u64
            );
            assert_eq!(
                crate::stats::count_nulls(all.view())
                    + crate::stats::analyze_statistics(all.view())
                        .unwrap()
                        .nan_count,
                all.view().len() as u64
            );
            assert!(build_scaled(all.view(), 3, AxisScale::Linear)
                .unwrap()
                .is_none());
        }
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(f64::NAN),
            None,
            Some(f64::NAN),
        ]));
        for (selected, total, nulls, nans) in [
            (Fixture::all(&array), 3, 1, 2),
            (Fixture::new(&array, vec![1, 2]), 2, 1, 1),
            (Fixture::new(&array, vec![]), 0, 0, 0),
        ] {
            let h = counts(selected.view(), 40).unwrap();
            assert_eq!(
                (
                    selected.view().len(),
                    crate::stats::count_nulls(selected.view()),
                    crate::stats::analyze_statistics(selected.view())
                        .unwrap()
                        .nan_count
                ),
                (total, nulls, nans)
            );
            assert!(h.counts.is_empty());
            assert!(h.min_label.is_none() && h.max_label.is_none());
        }
        for value in [f64::INFINITY, f64::NEG_INFINITY] {
            let array: ArrayRef =
                Arc::new(Float64Array::from(vec![Some(f64::NAN), None, Some(value)]));
            let all = Fixture::all(&array);
            assert!(matches!(
                counts(all.view(), 40),
                Err(crate::AnalysisError::NoFiniteValues)
            ));

            assert!(build_scaled(all.view(), 40, AxisScale::Linear)
                .unwrap()
                .is_none());
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
            verify_bins(Fixture::all(&array), 60);
            verify_bins(Fixture::new(&array, vec![1, (len - 1) as u32]), 3);
        }
        let adjacent: ArrayRef = Arc::new(Int64Array::from(vec![
            9_007_199_254_740_992,
            9_007_199_254_740_993,
        ]));
        let result = verify_bins(Fixture::all(&adjacent), 60);
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
        let result = verify_bins(Fixture::all(&tenths), 3);
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
            Arc::new(
                arrow::array::Decimal32Array::from(vec![-1, 0, 1])
                    .with_precision_and_scale(9, 2)
                    .unwrap(),
            ),
            Arc::new(
                arrow::array::Decimal64Array::from(vec![-1, 0, 1])
                    .with_precision_and_scale(18, 2)
                    .unwrap(),
            ),
        ];
        for array in arrays {
            let result = verify_bins(Fixture::all(&array), 60);
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
        verify_bins(Fixture::all(&extremes), 60);
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
        verify_bins(Fixture::all(&float16), 3);
        let result = verify_bins(Fixture::all(&float32), 3);
        assert_eq!(result.histogram.counts.iter().sum::<u64>(), 5);
        let infinities: ArrayRef = Arc::new(Float64Array::from(vec![
            f64::NEG_INFINITY,
            0.0,
            1.0,
            2.0,
            f64::INFINITY,
        ]));
        let result = verify_bins(Fixture::all(&infinities), 2);
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
            let array: ArrayRef = Arc::new(Float64Array::from(values));
            let result = verify_bins(Fixture::all(&array), bins);
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
            let all = Fixture::all(&array);
            assert!(build_scaled(all.view(), 60, AxisScale::Linear)
                .unwrap()
                .is_none());
        }
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(0..100));
        let all = Fixture::all(&array);
        assert!(build_scaled(all.view(), 0, AxisScale::Linear).is_err());
        let empty = Fixture::new(&array, vec![]);
        assert!(build_scaled(empty.view(), 1000, AxisScale::Linear)
            .unwrap()
            .is_none());
        let histogram = counts(empty.view(), 60).unwrap();
        assert!(histogram.counts.is_empty());
    }
}
