use arrow::array::Float64Array;

use super::*;

fn summary(values: &[f64]) -> NumericSummary {
    let array = Float64Array::from(values.to_vec());
    RealColumn::new(&array).summarize(&Selection::Range(0..values.len()))
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
