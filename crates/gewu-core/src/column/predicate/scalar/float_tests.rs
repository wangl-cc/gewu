use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};

use super::{equals, search};

#[test]
fn equality_parses_float32_without_double_rounding() {
    use arrow::array::Float32Array;

    // Just above the midpoint between 1.0_f32 and its successor. Parsing
    // through f64 rounds to the midpoint, then rounds the tie downward.
    let text = "1.0000000596046447753906250000001";
    assert_eq!(text.parse::<f64>().unwrap() as f32, 1.0);
    assert_eq!(text.parse::<f32>().unwrap(), 1.0_f32.next_up());
    let array: ArrayRef = Arc::new(Float32Array::from(vec![1.0, 1.0_f32.next_up()]));
    let mut matched = [false; 2];
    equals(&array, text).unwrap().evaluate(0..2, &mut matched);
    assert_eq!(matched, [false, true]);
}

fn matches(query: &str, value: f64) -> bool {
    let array: ArrayRef = Arc::new(Float64Array::from(vec![value]));
    let mut matched = [false];
    search(&array, query).unwrap().evaluate(0..1, &mut matched);
    matched[0]
}

#[test]
fn search_preserves_target_when_interval_collapses() {
    for query in ["3089.62000000000000", "-3089.62000000000000"] {
        let value: f64 = query.parse().unwrap();
        assert!(matches(query, value));
        assert!(!matches(query, value.next_down()));
        assert!(!matches(query, value.next_up()));
    }
    let value = 4_503_599_627_370_496.0_f64;
    assert!(matches("4503599627370496", value));
    assert!(matches("4503599627370496", value.next_down()));
    assert!(!matches("4503599627370496", value.next_up()));
    assert!(matches("0e-324", 0.0));
    assert!(matches("0e-324", -0.0));
    assert!(!matches("0e-324", 0.0_f64.next_up()));
    assert!(!matches("0e-324", 0.0_f64.next_down()));
}

#[test]
fn search_preserves_precision_and_half_open_boundaries() {
    assert!(matches("1.2", 1.23));
    assert!(!matches("1.20", 1.23));
    for (query, lo, hi) in [("2", 1.5_f64, 2.5_f64), ("-2", -2.5, -1.5)] {
        assert!(!matches(query, lo.next_down()));
        assert!(matches(query, lo));
        assert!(matches(query, hi.next_down()));
        assert!(!matches(query, hi));
    }
}
