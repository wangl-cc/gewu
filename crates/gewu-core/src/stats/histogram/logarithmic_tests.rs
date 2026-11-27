use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Decimal128Array, Decimal256Array, Float32Array, Float64Array,
    Int64Array, UInt64Array,
};

use super::*;
use crate::{col_view::test::Fixture, Filter, RangeHistogram};

fn verify(fixture: Fixture, bins: u32) -> Histogram {
    let view = fixture.view();
    let bins = NonZeroU32::new(bins).unwrap();
    let result: RangeHistogram = build(fixture.view(), bins).unwrap();
    let histogram: Histogram = build(fixture.view(), bins).unwrap();
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
    let mut memberships = vec![0; view.array().len()];
    for (range, &expected) in result.ranges.iter().zip(&histogram.counts) {
        assert!(range.min.is_some() && range.max.is_some());
        let predicate = view
            .column()
            .compile_filter(&Filter::range(
                (range.min.clone()).map(|b| b.value),
                (range.max.clone()).map(|b| b.value),
                false,
            ))
            .unwrap();
        let mut matched = vec![false; view.array().len()];
        predicate.evaluate(0..view.array().len(), &mut matched);
        assert_eq!(
            fixture.rows().iter().filter(|&row| matched[row]).count() as u64,
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
            + crate::stats::count_nulls(fixture.view())
            + crate::stats::analyze_statistics(fixture.view())
                .unwrap()
                .nan_count
            + histogram.non_positive_count
            + histogram.infinite_count,
        fixture.rows().len() as u64
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
    let histogram = verify(Fixture::all(&array), 3);
    assert_eq!(histogram.counts, [2, 2, 2]);
    assert_eq!(
        (histogram.non_positive_count, histogram.infinite_count),
        (2, 2)
    );
    assert_eq!((histogram.lo, histogram.hi), (Some(0.0), Some(3.0)));
    assert_eq!(
        (
            histogram.min_label.as_deref(),
            histogram.max_label.as_deref()
        ),
        (Some("1"), Some("1000"))
    );
    let histogram = verify(Fixture::new(&array, vec![2, 5, 6, 9, 11]), 3);
    assert_eq!(histogram.counts, [1, 0, 1]);
    assert_eq!((histogram.lo, histogram.hi), (Some(1.0), Some(3.0)));
    assert_eq!(
        (histogram.non_positive_count, histogram.infinite_count),
        (1, 1)
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
            arrow::array::Decimal32Array::from(vec![0, 1, 999_999_999])
                .with_precision_and_scale(9, 2)
                .unwrap(),
        ),
        Arc::new(
            arrow::array::Decimal64Array::from(vec![0, 1, 9_007_199_254_740_993])
                .with_precision_and_scale(18, 2)
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
        let histogram = verify(Fixture::all(&array), 40);
        assert!(histogram.lo.unwrap().is_finite());
        assert!(histogram.hi.unwrap().is_finite());
        assert!(histogram.lo < histogram.hi);
        assert!(!histogram.counts.is_empty());
    }
    let values = Float32Array::from(vec![0.0, 0.001, 1.0, 1000.0, f32::INFINITY]);
    let half = arrow::compute::cast(&values, &DataType::Float16).unwrap();
    verify(Fixture::all(&half), 20);

    // Rounding geometric cuts to an integer grid merges bins and can put
    // the final cut exactly at max. Its last singleton still needs width.
    let small: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 3]));
    let histogram = verify(Fixture::all(&small), 40);
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
    let histogram = verify(Fixture::all(&array), 5);
    assert!(histogram.counts.is_empty());
    assert_eq!((histogram.lo, histogram.hi), (None, None));
    assert_eq!(
        (histogram.non_positive_count, histogram.infinite_count),
        (2, 1)
    );
    let histogram = verify(Fixture::new(&array, vec![]), 5);
    assert!(histogram.counts.is_empty());
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![1, 10]));
    let dates = Fixture::all(&dates);
    assert!(build::<Histogram>(dates.view(), NonZeroU32::new(5).unwrap()).is_err());
}
