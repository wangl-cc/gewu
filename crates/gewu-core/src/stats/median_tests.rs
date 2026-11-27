use std::sync::Arc;

use arrow::array::{
    Date32Array, Decimal128Array, Decimal256Array, Float64Array, Int64Array,
    TimestampNanosecondArray, UInt64Array,
};

use super::*;

fn view_label(view: crate::col_view::ColView<'_>) -> Option<String> {
    label(Numeric::new(view.array()).unwrap(), view.selection()).unwrap()
}

fn median(array: ArrayRef) -> Option<String> {
    let all = crate::col_view::test::Fixture::all(&array);
    view_label(all.view())
}

#[test]
fn medians_keep_native_precision_and_fractional_midpoints() {
    assert_eq!(
        median(Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX]))).as_deref(),
        Some("18446744073709551614.5")
    );
    assert_eq!(
        median(Arc::new(Int64Array::from(vec![Some(-1), None, Some(0)]))).as_deref(),
        Some("-0.5")
    );
    assert_eq!(
        median(Arc::new(Int64Array::from(vec![i64::MIN, i64::MAX]))).as_deref(),
        Some("-0.5")
    );
    assert_eq!(
        median(Arc::new(
            Decimal128Array::from(vec![123, 124])
                .with_precision_and_scale(10, 2)
                .unwrap()
        ))
        .as_deref(),
        Some("1.235")
    );
    assert_eq!(
        median(Arc::new(
            Decimal128Array::from(vec![0, 1])
                .with_precision_and_scale(10, -2)
                .unwrap()
        ))
        .as_deref(),
        Some("50")
    );
    let large: i256 =
        "9999999999999999999999999999999999999999999999999999999999999999999999999999"
            .parse()
            .unwrap();
    assert_eq!(
        median(Arc::new(
            Decimal256Array::from(vec![large - i256::ONE, large])
                .with_precision_and_scale(76, 0)
                .unwrap()
        ))
        .unwrap(),
        format!("{}.5", large - i256::ONE)
    );
    assert_eq!(
        median(Arc::new(Float64Array::from(vec![None, Some(f64::NAN)]))),
        None
    );
    assert_eq!(
        median(Arc::new(Float64Array::from(vec![
            f64::NEG_INFINITY,
            f64::INFINITY
        ]))),
        None
    );
    assert_eq!(
        median(Arc::new(Float64Array::from(vec![f64::MAX, f64::MAX]))),
        Some(f64::MAX.to_string())
    );
    assert_eq!(
        median(Arc::new(Float64Array::from(vec![0.0, f64::INFINITY]))).as_deref(),
        Some("inf")
    );
    let tiny = f64::from_bits(1);
    for (a, b, expected) in [(tiny, 2.0 * tiny, 2.0 * tiny), (-tiny, 2.0 * tiny, 0.0)] {
        assert_eq!(
            median(Arc::new(Float64Array::from(vec![a, b]))),
            Some(expected.to_string())
        );
    }
}

#[test]
fn temporal_medians_keep_fractional_units_and_timezone() {
    assert_eq!(
        median(Arc::new(Date32Array::from(vec![0, 1]))).as_deref(),
        Some("1970-01-01T12:00:00")
    );
    assert_eq!(
        median(Arc::new(
            TimestampNanosecondArray::from(vec![0, 1]).with_timezone("UTC")
        ))
        .as_deref(),
        Some("1970-01-01T00:00:00.0000000005Z")
    );
    assert_eq!(
        median(Arc::new(
            TimestampNanosecondArray::from(vec![-1, 0]).with_timezone("UTC")
        ))
        .as_deref(),
        Some("1969-12-31T23:59:59.9999999995Z")
    );
    assert_eq!(
        median(Arc::new(
            TimestampSecondArray::from(vec![0, 1]).with_timezone("Europe/London")
        ))
        .as_deref(),
        Some("1970-01-01T01:00:00.5+01:00")
    );
}

#[test]
fn filtered_and_reordered_selections_match_sorted_median() {
    let array: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 100, 101]));
    let selected = crate::col_view::test::Fixture::new(&array, vec![3, 1]);
    assert_eq!(view_label(selected.view()).as_deref(), Some("51.5"));

    let odd = crate::col_view::test::Fixture::new(&array, vec![3, 0, 2]);
    assert_eq!(view_label(odd.view()).as_deref(), Some("100"));
}
