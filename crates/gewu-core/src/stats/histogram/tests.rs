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
        let array: ArrayRef = Arc::new(Float64Array::from(vec![Some(f64::NAN), None, Some(value)]));
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
