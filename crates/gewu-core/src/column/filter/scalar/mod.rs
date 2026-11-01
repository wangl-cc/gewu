//! Scalar type semantics. Preparation borrows the native array and parses
//! inputs once; evaluating a predicate is the executor's responsibility.

pub(super) mod decimal;
pub(super) mod float;
pub(super) mod integer;
pub(super) mod temporal;

use arrow::array::{Array, ArrayRef, AsArray};

use super::range::{prepare_range, Bound};
use crate::{predicate::Predicate, selection::RowSelection, Error, LogicalType, Result};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Result<Option<Predicate<'a>>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(match LogicalType::from_arrow(array.data_type()) {
        LogicalType::Integer { .. } => integer::search(array, text),
        LogicalType::Float { .. } => float::search(array, text),
        LogicalType::Decimal { .. } => decimal::search(array, text),
        LogicalType::Boolean => boolean(text).map(|target| bool_predicate(array, target)),
        LogicalType::Date
        | LogicalType::Timestamp { .. }
        | LogicalType::Time
        | LogicalType::Duration { .. } => {
            return temporal::search(array, text);
        }
        _ => {
            return Err(Error::InvalidArgument(format!(
                "{} is not a scalar query type",
                array.data_type()
            )));
        }
    })
}

pub(super) fn equals<'a>(array: &'a ArrayRef, text: &str) -> Result<Predicate<'a>> {
    let text = text.trim();
    match LogicalType::from_arrow(array.data_type()) {
        LogicalType::Boolean => boolean(text)
            .map(|target| bool_predicate(array, target))
            .ok_or_else(|| Error::InvalidArgument(format!("cannot read {text:?} as a boolean"))),
        LogicalType::Float { .. } => float::equals(array, text),
        _ => {
            let bound = Bound::parse(array.data_type(), text)?;
            prepare_range(array, Some(&bound), Some(&bound))
        }
    }
}

fn boolean(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn bool_predicate(array: &ArrayRef, target: bool) -> Predicate<'_> {
    let array = array.as_boolean();
    let has_nulls = array.null_count() > 0;
    Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                for (i, row) in range.enumerate() {
                    output[i] = (!has_nulls || array.is_valid(row)) && array.value(row) == target;
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                for (i, row) in range.enumerate() {
                    output[i] = mask[i]
                        && (!has_nulls || array.is_valid(row))
                        && array.value(row) == target;
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = (!has_nulls || array.is_valid(row)) && array.value(row) == target;
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = mask[i]
                        && (!has_nulls || array.is_valid(row))
                        && array.value(row) == target;
                }
            }
        }
        Ok(())
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            BooleanArray, Date32Array, Decimal128Array, Decimal256Array, Float32Array,
            Float64Array, Int64Array, Int8Array, UInt64Array,
        },
        datatypes::i256,
    };

    use super::*;
    use crate::engine::CancelToken;

    fn matches(predicate: Predicate<'_>, len: usize) -> Vec<bool> {
        let mut matched = vec![false; len];
        predicate
            .evaluate(0..len, &mut matched, &CancelToken::never())
            .unwrap();
        matched
    }

    #[test]
    fn decimal_equality_and_ranges_keep_large_coefficients_exact() {
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![
                Some(9_007_199_254_740_992),
                Some(9_007_199_254_740_993),
                Some(9_007_199_254_740_994),
                None,
            ])
            .with_precision_and_scale(38, 0)
            .unwrap(),
        );
        assert_eq!(matches(equals(&array, "9007199254740993").unwrap(), 4), [
            false, true, false, false
        ]);
        let lower = Bound::parse(array.data_type(), "9007199254740992.1").unwrap();
        let upper = Bound::parse(array.data_type(), "9007199254740993.9").unwrap();
        assert_eq!(
            matches(
                prepare_range(&array, Some(&lower), Some(&upper)).unwrap(),
                4
            ),
            [false, true, false, false]
        );
        let lower = Bound::parse(array.data_type(), "9007199254740993.1").unwrap();
        let upper = Bound::parse(array.data_type(), "9007199254740993.0").unwrap();
        assert!(prepare_range(&array, Some(&lower), Some(&upper)).is_err());

        let n: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let array: ArrayRef = Arc::new(
            Decimal256Array::from(vec![n, n + i256::ONE])
                .with_precision_and_scale(76, 0)
                .unwrap(),
        );
        assert_eq!(matches(equals(&array, &n.to_string()).unwrap(), 2), [
            true, false
        ]);
        assert_eq!(
            matches(search(&array, &n.to_string()).unwrap().unwrap(), 2),
            [true, false]
        );
    }

    #[test]
    fn decimal_search_precision_differs_from_equality_and_respects_scale() {
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![115, 120, 123, 125, -125, -123, -115])
                .with_precision_and_scale(20, 2)
                .unwrap(),
        );
        assert_eq!(matches(search(&array, "1.2").unwrap().unwrap(), 7), [
            true, true, true, false, false, false, false
        ]);
        assert_eq!(matches(search(&array, "1.20").unwrap().unwrap(), 7), [
            false, true, false, false, false, false, false
        ]);
        assert_eq!(matches(equals(&array, "1.2").unwrap(), 7), [
            false, true, false, false, false, false, false
        ]);
        assert_eq!(matches(search(&array, "-1.2").unwrap().unwrap(), 7), [
            false, false, false, false, true, true, false
        ]);
        assert_eq!(matches(equals(&array, "1.201").unwrap(), 7), [false; 7]);
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![12, 13])
                .with_precision_and_scale(20, -2)
                .unwrap(),
        );
        assert_eq!(matches(search(&array, "1.2e3").unwrap().unwrap(), 2), [
            true, false
        ]);
        assert_eq!(matches(equals(&array, "1200").unwrap(), 2), [true, false]);
    }

    #[test]
    fn decimal_extreme_input_precision_and_magnitude_do_not_round_or_overflow() {
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![-1, 0, 1])
                .with_precision_and_scale(20, 0)
                .unwrap(),
        );
        let lower = Bound::parse(array.data_type(), "1e1000000").unwrap();
        assert_eq!(
            matches(prepare_range(&array, Some(&lower), None).unwrap(), 3),
            [false; 3]
        );
        let lower = Bound::parse(array.data_type(), "-1e1000000").unwrap();
        let upper = Bound::parse(array.data_type(), "1e1000000").unwrap();
        assert_eq!(
            matches(
                prepare_range(&array, Some(&lower), Some(&upper)).unwrap(),
                3
            ),
            [true; 3]
        );
        assert_eq!(matches(equals(&array, "1.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000001").unwrap(), 3), [false; 3]);
        assert_eq!(
            matches(search(&array, "0e-1000000").unwrap().unwrap(), 3),
            [false, true, false]
        );
        for input in ["NaN", "inf", "1e", "1.2.3", "--2", ""] {
            assert!(equals(&array, input).is_err(), "{input}");
            assert!(search(&array, input).unwrap().is_none(), "{input}");
        }
    }

    #[test]
    fn integer_ranges_are_exact_and_do_not_narrow_inputs_to_column_width() {
        let array: ArrayRef = Arc::new(Int8Array::from(vec![Some(-128), Some(2), Some(127), None]));
        let lower = Bound::parse(array.data_type(), "1.1").unwrap();
        let upper = Bound::parse(array.data_type(), "2.9").unwrap();
        assert_eq!(
            matches(
                prepare_range(&array, Some(&lower), Some(&upper)).unwrap(),
                4
            ),
            [false, true, false, false]
        );
        let lower = Bound::parse(array.data_type(), "128").unwrap();
        assert_eq!(
            matches(prepare_range(&array, Some(&lower), None).unwrap(), 4),
            [false; 4]
        );
        assert!(search(&array, "2.0").unwrap().is_none());
        let array: ArrayRef = Arc::new(Int64Array::from(vec![
            9_007_199_254_740_992,
            9_007_199_254_740_993,
        ]));
        let lower = Bound::parse(array.data_type(), "9007199254740992.1").unwrap();
        assert_eq!(
            matches(prepare_range(&array, Some(&lower), None).unwrap(), 2),
            [false, true]
        );
        let array: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX - 1, u64::MAX]));
        assert_eq!(
            matches(equals(&array, "18446744073709551615").unwrap(), 2),
            [false, true]
        );
    }

    #[test]
    fn native_floats_preserve_search_semantics_and_parse_equality_to_their_type() {
        let array: ArrayRef = Arc::new(Float32Array::from(vec![
            Some(1.2),
            Some(1.23),
            Some(f32::NAN),
            None,
        ]));
        assert_eq!(matches(search(&array, "1.2").unwrap().unwrap(), 4), [
            true, true, false, false
        ]);
        assert_eq!(matches(equals(&array, "1.2").unwrap(), 4), [
            true, false, false, false
        ]);
        assert_eq!(matches(search(&array, "NaN").unwrap().unwrap(), 4), [
            false, false, true, false
        ]);
        assert!(Bound::parse(array.data_type(), "NaN").is_err());
        let lower = Bound::parse(array.data_type(), "2").unwrap();
        let upper = Bound::parse(array.data_type(), "1").unwrap();
        assert!(prepare_range(&array, Some(&lower), Some(&upper)).is_err());
        let lower = Bound::parse(array.data_type(), "9007199254740993").unwrap();
        let upper = Bound::parse(array.data_type(), "9007199254740992").unwrap();
        assert!(prepare_range(&array, Some(&lower), Some(&upper)).is_err());
        assert_eq!(matches(equals(&array, "1e100").unwrap(), 4), [false; 4]);
        let array: ArrayRef = Arc::new(Float64Array::from(vec![f64::INFINITY, f64::NEG_INFINITY]));
        assert_eq!(matches(search(&array, "+inf").unwrap().unwrap(), 2), [
            true, false
        ]);
        assert!(search(&array, "1e999").unwrap().is_none());
        assert_eq!(matches(equals(&array, "1e999").unwrap(), 2), [false, false]);
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            9_007_199_254_740_992.0,
            9_007_199_254_740_994.0,
        ]));
        let lower = Bound::parse(array.data_type(), "9007199254740993").unwrap();
        assert_eq!(
            matches(prepare_range(&array, Some(&lower), None).unwrap(), 2),
            [false, true]
        );
        let edge = u64::MAX as f64;
        let array: ArrayRef = Arc::new(Float64Array::from(vec![edge.next_down(), edge]));
        let lower = Bound::parse(array.data_type(), "18446744073709551615").unwrap();
        assert_eq!(
            matches(prepare_range(&array, None, Some(&lower)).unwrap(), 2),
            [true, false]
        );
        let lower = Bound::parse(array.data_type(), "18446744073709551615").unwrap();
        let upper = Bound::parse(array.data_type(), "18446744073709551615").unwrap();
        assert_eq!(
            matches(
                prepare_range(&array, Some(&lower), Some(&upper)).unwrap(),
                2
            ),
            [false; 2]
        );
    }

    #[test]
    fn axis_projection_is_approximate_and_does_not_limit_exact_filters() {
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![9_007_199_254_740_992, 9_007_199_254_740_993])
                .with_precision_and_scale(38, 0)
                .unwrap(),
        );
        assert_eq!(
            Bound::parse(array.data_type(), "9007199254740992")
                .unwrap()
                .axis_value(),
            Bound::parse(array.data_type(), "9007199254740993")
                .unwrap()
                .axis_value()
        );
        assert_eq!(matches(equals(&array, "9007199254740993").unwrap(), 2), [
            false, true
        ]);
        assert_eq!(
            Bound::parse(array.data_type(), "1e999")
                .unwrap()
                .axis_value(),
            f64::INFINITY
        );
        let lower = Bound::parse(array.data_type(), "1e999").unwrap();
        assert_eq!(
            matches(prepare_range(&array, Some(&lower), None).unwrap(), 2),
            [false; 2]
        );
        for text in ["NaN", "inf", "1.2.3", "cat"] {
            assert!(Bound::parse(array.data_type(), text).is_err());
        }
        let array: ArrayRef = Arc::new(Float32Array::from(vec![1.0]));
        assert_eq!(
            Bound::parse(array.data_type(), "inf").unwrap().axis_value(),
            f64::INFINITY
        );
        assert!(Bound::parse(array.data_type(), "NaN").is_err());
        let array: ArrayRef = Arc::new(Int8Array::from(vec![1]));
        assert_eq!(
            Bound::parse(array.data_type(), "1000")
                .unwrap()
                .axis_value(),
            1000.0
        );
        assert_eq!(
            Bound::parse(array.data_type(), "1.5").unwrap().axis_value(),
            1.5
        );
    }

    #[test]
    fn temporal_axes_and_exact_filters_share_units_without_float_roundtrips() {
        use arrow::array::{
            Date64Array, DurationMillisecondArray, Time32SecondArray, Time64NanosecondArray,
            TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
        };
        let cases = [
            (
                Arc::new(Date32Array::from(vec![1])) as ArrayRef,
                "1970-01-02",
                1.0,
            ),
            (
                Arc::new(Date64Array::from(vec![86_400_000])),
                "1970-01-02",
                86_400_000.0,
            ),
            (
                Arc::new(Time32SecondArray::from(vec![3_600])),
                "01:00:00",
                3_600.0,
            ),
            (
                Arc::new(Time64NanosecondArray::from(vec![3_600_000_000_000])),
                "01:00:00",
                3_600_000_000_000.0,
            ),
            (
                Arc::new(TimestampSecondArray::from(vec![86_400]).with_timezone("UTC")),
                "1970-01-02T00:00:00",
                86_400.0,
            ),
            (
                Arc::new(TimestampMillisecondArray::from(vec![86_400_000])),
                "1970-01-02T00:00:00",
                86_400_000.0,
            ),
            (
                Arc::new(DurationMillisecondArray::from(vec![123])),
                "123",
                123.0,
            ),
        ];
        for (array, text, coordinate) in cases {
            assert_eq!(
                Bound::parse(array.data_type(), text).unwrap().axis_value(),
                coordinate,
                "{}",
                array.data_type()
            );
            assert_eq!(
                matches(equals(&array, text).unwrap(), 1),
                [true],
                "{}",
                array.data_type()
            );
        }
        let array: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![
            9_007_199_254_740_992,
            9_007_199_254_740_993,
        ]));
        let text = "1970-04-15T05:59:59.254740993";
        assert_eq!(
            Bound::parse(array.data_type(), text).unwrap().axis_value(),
            9_007_199_254_740_992.0
        );
        assert_eq!(matches(equals(&array, text).unwrap(), 2), [false, true]);
        let lower = Bound::parse(array.data_type(), "1970-04-16").unwrap();
        let upper = Bound::parse(array.data_type(), "1970-04-15").unwrap();
        assert!(prepare_range(&array, Some(&lower), Some(&upper)).is_err());
    }

    #[test]
    fn invalid_global_search_skips_scalar_columns_but_explicit_filters_fail() {
        for array in [
            Arc::new(Int64Array::from(vec![1])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0])),
            Arc::new(BooleanArray::from(vec![true])),
            Arc::new(Date32Array::from(vec![1])),
        ] {
            assert!(search(&array, "cat").unwrap().is_none());
            assert!(equals(&array, "cat").is_err());
        }
        let array: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), Some(false), None]));
        assert_eq!(matches(equals(&array, " TRUE ").unwrap(), 3), [
            true, false, false
        ]);
    }
}
