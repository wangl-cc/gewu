//! Geometric cuts are prepared once, then native values decide bin membership.
//! Logarithms affect cut placement and plotting only, never filter comparison.

use std::num::NonZeroU32;

use arrow::{
    array::{downcast_integer_array, Array, AsArray, PrimitiveArray},
    datatypes::*,
};
use num_traits::ToPrimitive;

use super::{Histogram, Output, RangeBounds};
use crate::{
    col_view::{ColView, Selection},
    scalar::Bound,
    stats::{scan::for_each_valid, Result},
    AxisScale,
};

pub(super) fn build<H: Output>(view: ColView<'_>, bins: NonZeroU32) -> Result<H> {
    let array = view.array();
    let bins = bins.get() as usize;
    let selection = view.selection();
    match array.data_type() {
        t if t.is_integer() => {
            downcast_integer_array!(array => integer(array, selection, bins), _ => unreachable!("integer dispatch"))
        }
        DataType::Float16 => floating(array.as_primitive::<Float16Type>(), selection, bins),
        DataType::Float32 => floating(array.as_primitive::<Float32Type>(), selection, bins),
        DataType::Float64 => floating(array.as_primitive::<Float64Type>(), selection, bins),
        DataType::Decimal32(_, scale) => decimal(
            array.as_primitive::<Decimal32Type>(),
            selection,
            bins,
            *scale,
            |v| i256::from_i128(i128::from(v)),
            |v| v.to_i128().expect("native coefficient domain") as i32,
        ),
        DataType::Decimal64(_, scale) => decimal(
            array.as_primitive::<Decimal64Type>(),
            selection,
            bins,
            *scale,
            |v| i256::from_i128(i128::from(v)),
            |v| v.to_i128().expect("native coefficient domain") as i64,
        ),
        DataType::Decimal128(_, scale) => decimal(
            array.as_primitive::<Decimal128Type>(),
            selection,
            bins,
            *scale,
            i256::from_i128,
            |v| v.to_i128().expect("native coefficient domain"),
        ),
        DataType::Decimal256(_, scale) => decimal(
            array.as_primitive::<Decimal256Type>(),
            selection,
            bins,
            *scale,
            |v| v,
            |v| v,
        ),
        data_type => Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Distribution,
            data_type: data_type.clone(),
        }),
    }
}

fn positive_extrema<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    positive: impl Fn(T::Native) -> bool,
) -> Option<(T::Native, T::Native)>
where
    T::Native: PartialOrd,
{
    let mut extrema: Option<(T::Native, T::Native)> = None;
    for_each_valid(array, selection, |_, value| {
        if positive(value) {
            extrema = Some(match extrema {
                Some((lo, hi)) => (
                    if value < lo { value } else { lo },
                    if value > hi { value } else { hi },
                ),
                None => (value, value),
            });
        }
    });
    extrema
}

/// The native grid can collapse adjacent geometric cuts. Keep only increasing
/// cuts in the source domain rather than create empty or reversed ranges.
fn cuts<N: Copy + PartialOrd>(
    min: N,
    max: N,
    log_min: f64,
    log_max: f64,
    bins: usize,
    native: impl Fn(f64) -> Option<N>,
) -> Vec<N> {
    let mut cuts = Vec::with_capacity(bins.saturating_sub(1));
    if log_min < log_max {
        for index in 1..bins {
            let exponent = log_min + (log_max - log_min) * (index as f64 / bins as f64);
            let Some(cut) = native(10_f64.powf(exponent)) else {
                continue;
            };
            if cut > min && cut <= max && cuts.last().is_none_or(|&last| last < cut) {
                cuts.push(cut);
            }
        }
    }
    cuts
}

fn integer<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
) -> Result<H>
where
    T::Native: Into<i128> + TryFrom<i128> + Ord,
{
    let extrema = positive_extrema(array, selection, |value| value.into() > 0);
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, selection, empty(), |_| Class::NonPositive),
            || Ok(Vec::new()),
        );
    };
    let (lo, hi) = ((min.into() as f64).log10(), (max.into() as f64).log10());
    let cuts = cuts(min, max, lo, hi, bins, |value| {
        T::Native::try_from(value.ceil() as i128).ok()
    });
    let bound = |value: T::Native| {
        if array.data_type().is_unsigned_integer() {
            Ok(Bound::from_u64(array.data_type(), value.into() as u64)?)
        } else {
            Ok(Bound::from_i64(array.data_type(), value.into() as i64)?)
        }
    };
    let lower = bound(min)?;
    let upper = bound(max)?;
    let edges = plot_edges(
        lo,
        ((max.into() + 1) as f64).log10(),
        cuts.iter().map(|&value| (value.into() as f64).log10()),
    );
    let histogram = count(
        array,
        selection,
        with_domain(edges, &lower, &upper),
        |value| {
            if value.into() <= 0 {
                Class::NonPositive
            } else {
                Class::Bin(cuts.partition_point(|&cut| cut <= value))
            }
        },
    );
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, bound, |value| {
            let previous = T::Native::try_from(value.into() - 1)
                .ok()
                .expect("positive cut predecessor");
            bound(previous)
        })
    })
}

fn decimal<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
    scale: i8,
    widen: impl Fn(T::Native) -> i256,
    narrow: impl Fn(i256) -> T::Native,
) -> Result<H>
where
    T::Native: Ord,
{
    let extrema = positive_extrema(array, selection, |value| widen(value) > i256::ZERO);
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, selection, empty(), |_| Class::NonPositive),
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
    let cuts = cuts(min, max, lo, hi, bins, |value| {
        let coefficient = i256::from_f64(value.ceil())?;
        (coefficient > min_coefficient && coefficient <= max_coefficient)
            .then(|| narrow(coefficient))
    });
    let bound = |value| Ok(Bound::from_decimal(array.data_type(), widen(value), scale)?);
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
    );
    let histogram = count(
        array,
        selection,
        with_domain(edges, &lower, &upper),
        |value| {
            if widen(value) <= i256::ZERO {
                Class::NonPositive
            } else {
                Class::Bin(cuts.partition_point(|&cut| cut <= value))
            }
        },
    );
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, bound, |value| {
            Ok(Bound::from_decimal(
                array.data_type(),
                widen(value) - i256::ONE,
                scale,
            )?)
        })
    })
}

fn floating<T: ArrowPrimitiveType, H: Output>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    bins: usize,
) -> Result<H>
where
    T::Native: Into<f64> + PartialOrd,
{
    let extrema = positive_extrema(array, selection, |value| {
        let value = value.into();
        value > 0.0 && value.is_finite()
    });
    let Some((min, max)) = extrema else {
        return H::finish(
            count(array, selection, empty(), |value| {
                classify_float(value.into(), &[])
            }),
            || Ok(Vec::new()),
        );
    };
    let (min, max) = (min.into(), max.into());
    let (lo, hi) = (min.log10(), max.log10());
    let cuts = cuts(min, max, lo, hi, bins, Some);
    let bound = |value| Ok(Bound::from_f64(array.data_type(), value)?);
    let lower = bound(min)?;
    let upper = bound(max)?;
    // Include a final cut that rounds to max without giving its singleton bin
    // zero width. At f64::MAX expand in log space to keep coordinates finite.
    let display_hi = if max < f64::MAX {
        max.next_up().log10()
    } else {
        hi.next_up()
    };
    let edges = plot_edges(lo, display_hi, cuts.iter().map(|value| value.log10()));
    let histogram = count(
        array,
        selection,
        with_domain(edges, &lower, &upper),
        |value| classify_float(value.into(), &cuts),
    );
    H::finish(histogram, || {
        finite_ranges(&cuts, lower, upper, bound, |value| bound(value.next_down()))
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

fn empty() -> Histogram {
    Histogram {
        scale: AxisScale::Log10,
        lo: None,
        hi: None,
        min_label: None,
        max_label: None,
        integer: false,
        counts: Vec::new(),
        bin_edges: Vec::new(),
        non_positive_count: 0,
        infinite_count: 0,
    }
}

/// Preserve the unequal widths left by rounding geometric cuts onto a native
/// grid. Adjacent exact values can share a logarithm at f64 precision; nudge
/// only those display coordinates, never the native cut or filter endpoint.
fn plot_edges(lo: f64, hi: f64, cuts: impl ExactSizeIterator<Item = f64>) -> Vec<f64> {
    let mut edges = Vec::with_capacity(cuts.len() + 2);
    edges.push(lo);
    let mut previous = lo;
    for edge in cuts.chain(std::iter::once(hi)) {
        let edge = if edge <= previous {
            previous.next_up()
        } else {
            edge
        };
        edges.push(edge);
        previous = edge;
    }
    edges
}

fn with_domain(bin_edges: Vec<f64>, min: &Bound, max: &Bound) -> Histogram {
    Histogram {
        lo: bin_edges.first().copied(),
        hi: bin_edges.last().copied(),
        min_label: Some(min.display().to_owned()),
        max_label: Some(max.display().to_owned()),
        counts: vec![0; bin_edges.len() - 1],
        bin_edges,
        ..empty()
    }
}

fn count<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    mut histogram: Histogram,
    classify: impl Fn(T::Native) -> Class,
) -> Histogram {
    for_each_valid(array, selection, |_, value| match classify(value) {
        Class::Bin(bin) => histogram.counts[bin] += 1,
        Class::NonPositive => histogram.non_positive_count += 1,
        Class::Infinite => histogram.infinite_count += 1,
        Class::NaN => {}
    });
    histogram
}

fn finite_ranges<N: Copy>(
    cuts: &[N],
    min: Bound,
    max: Bound,
    lower: impl Fn(N) -> Result<Bound>,
    upper: impl Fn(N) -> Result<Bound>,
) -> Result<Vec<RangeBounds>> {
    (0..=cuts.len())
        .map(|index| {
            Ok(RangeBounds {
                min: Some(
                    if index == 0 {
                        min.clone()
                    } else {
                        lower(cuts[index - 1])?
                    }
                    .into(),
                ),
                max: Some(
                    if index == cuts.len() {
                        max.clone()
                    } else {
                        upper(cuts[index])?
                    }
                    .into(),
                ),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "logarithmic_tests.rs"]
mod tests;
