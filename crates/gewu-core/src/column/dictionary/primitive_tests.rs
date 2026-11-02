use std::sync::Arc;

use arrow::array::ArrayRef;

use super::{super::Dictionary, *};
use crate::Cancellation;

mod float {
    use arrow::{
        array::Float64Array,
        datatypes::{DataType, Float64Type},
    };

    use super::*;

    #[test]
    fn signed_zeros_canonicalize_to_positive_zero() {
        assert_eq!(Float64Type::canonical(0.0), (1 << 63, 0.0));
        assert_eq!(Float64Type::canonical(-0.0), (1 << 63, 0.0));
    }

    #[test]
    fn nan_payloads_canonicalize_to_standard_nan() {
        let payloads = [
            f64::NAN,
            f64::from_bits(f64::NAN.to_bits() | 1),
            f64::from_bits(0x7ff8_0000_0000_0001),
            f64::from_bits(0xfff8_0000_0000_0000),
        ];
        for value in payloads {
            let (key, native) = Float64Type::canonical(value);
            assert_eq!(key, f64::NAN.to_bits() | (1 << 63));
            assert!(native.is_nan());
        }
    }

    #[test]
    fn float_keys_order_like_the_values() {
        let values = [
            f64::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            1.0,
            f64::INFINITY,
            f64::NAN,
        ];
        let keys: Vec<u64> = values
            .iter()
            .copied()
            .map(|v| Float64Type::canonical(v).0)
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]), "{keys:?}");
        assert_eq!(keys[2], keys[3]);
        assert!(keys[0] < keys[1] && keys[4] < keys[5] && keys[5] < keys[6]);
    }

    #[test]
    fn float_values_group_signed_zero_and_nan_payloads() {
        let source = Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(f64::from_bits(0xfff8_2000_0000_0000)),
            None,
            Some(1.0),
        ]);
        for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
            let array = arrow::compute::cast(&source, &data_type).unwrap();
            let dictionary = Dictionary::build(&array, &Cancellation::default())
                .unwrap()
                .unwrap();
            assert_eq!(dictionary.n_values(), 3, "{data_type:?}");
            assert!((dictionary.null_count() > 0), "{data_type:?}");

            // Signed zeroes share the same code; different NaN payloads share the same code.
            assert_eq!(
                dictionary.code_at(0),
                dictionary.code_at(1),
                "{data_type:?}"
            );
            assert_eq!(
                dictionary.code_at(2),
                dictionary.code_at(3),
                "{data_type:?}"
            );
            assert_eq!(
                dictionary.code_at(2),
                dictionary.code_at(4),
                "{data_type:?}"
            );
            assert!(dictionary.is_null(5), "{data_type:?}");
            assert_ne!(
                dictionary.code_at(0),
                dictionary.code_at(2),
                "{data_type:?}"
            );
            assert_ne!(
                dictionary.code_at(0),
                dictionary.code_at(6),
                "{data_type:?}"
            );
        }
    }
}

mod integer {
    use arrow::{
        array::{Int32Array, Int64Array, UInt64Array},
        compute::cast,
        datatypes::DataType,
    };

    use super::*;

    #[test]
    fn numbers_group_distinct_values() {
        let a: ArrayRef = Arc::new(Int32Array::from(vec![10, -3, 2, 10]));
        let dictionary = Dictionary::build(&a, &Cancellation::default())
            .unwrap()
            .unwrap();
        assert_eq!(dictionary.n_values(), 3);
    }

    #[test]
    fn integer_arrays_group_distinct_values() {
        let signed: ArrayRef = Arc::new(Int64Array::from(vec![
            Some(-3),
            Some(7),
            Some(-3),
            None,
            Some(2),
        ]));
        for data_type in [
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
        ] {
            let array = cast(&signed, &data_type).expect("integer cast");
            let dictionary = Dictionary::build(&array, &Cancellation::default())
                .unwrap()
                .expect("encoded");
            assert_eq!(dictionary.n_values(), 3, "{data_type}");
            assert!((dictionary.null_count() > 0));
            assert_eq!(dictionary.code_at(0), dictionary.code_at(2), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(1), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(4), "{data_type}");
            assert!(dictionary.is_null(3), "{data_type}");
        }

        let unsigned: ArrayRef = Arc::new(UInt64Array::from(vec![
            Some(3),
            Some(7),
            Some(3),
            None,
            Some(2),
        ]));
        for data_type in [
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
        ] {
            let array = cast(&unsigned, &data_type).expect("unsigned cast");
            let dictionary = Dictionary::build(&array, &Cancellation::default())
                .unwrap()
                .expect("encoded");
            assert_eq!(dictionary.n_values(), 3, "{data_type}");
            assert!((dictionary.null_count() > 0));
            assert_eq!(dictionary.code_at(0), dictionary.code_at(2), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(1), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(4), "{data_type}");
            assert!(dictionary.is_null(3), "{data_type}");
        }
    }
}

mod temporal {
    use arrow::{
        array::{
            Date32Array, DurationSecondArray, Time32MillisecondArray, TimestampMicrosecondArray,
        },
        datatypes::{DataType, TimeUnit},
    };

    use super::*;

    #[test]
    fn temporal_arrays_group_distinct_values() {
        let cases: [(ArrayRef, DataType); 4] = [
            (
                Arc::new(Date32Array::from(vec![
                    Some(3),
                    Some(1),
                    Some(3),
                    None,
                    Some(2),
                ])),
                DataType::Date32,
            ),
            (
                Arc::new(Time32MillisecondArray::from(vec![
                    Some(3_000),
                    Some(1_000),
                    Some(3_000),
                    None,
                    Some(2_000),
                ])),
                DataType::Time32(TimeUnit::Millisecond),
            ),
            (
                Arc::new(TimestampMicrosecondArray::from(vec![
                    Some(3_000_000),
                    Some(1_000_000),
                    Some(3_000_000),
                    None,
                    Some(2_000_000),
                ])),
                DataType::Timestamp(TimeUnit::Microsecond, None),
            ),
            (
                Arc::new(DurationSecondArray::from(vec![
                    Some(3),
                    Some(1),
                    Some(3),
                    None,
                    Some(2),
                ])),
                DataType::Duration(TimeUnit::Second),
            ),
        ];
        for (array, data_type) in cases {
            assert_eq!(array.data_type(), &data_type);
            let dictionary = Dictionary::build(&array, &Cancellation::default())
                .unwrap()
                .expect("encoded");
            assert_eq!(dictionary.n_values(), 3, "{data_type}");
            assert!((dictionary.null_count() > 0));
            assert_eq!(dictionary.code_at(0), dictionary.code_at(2), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(1), "{data_type}");
            assert_ne!(dictionary.code_at(0), dictionary.code_at(4), "{data_type}");
            assert!(dictionary.is_null(3), "{data_type}");
        }
    }
}

mod decimal {
    use arrow::{
        array::{Decimal128Array, Decimal256Array},
        datatypes::i256,
    };

    use super::*;

    #[test]
    fn decimal_arrays_group_distinct_values() {
        let d128: ArrayRef = Arc::new(
            Decimal128Array::from(vec![Some(123_i128), Some(-45), Some(123), None, Some(0)])
                .with_precision_and_scale(10, 2)
                .unwrap(),
        );
        let dictionary = Dictionary::build(&d128, &Cancellation::default())
            .unwrap()
            .expect("encoded");
        assert_eq!(dictionary.n_values(), 3);
        assert!((dictionary.null_count() > 0));
        assert_eq!(dictionary.code_at(0), dictionary.code_at(2));
        assert_ne!(dictionary.code_at(0), dictionary.code_at(1));
        assert_ne!(dictionary.code_at(0), dictionary.code_at(4));
        assert!(dictionary.is_null(3));

        let d256: ArrayRef = Arc::new(
            Decimal256Array::from(vec![
                Some(i256::from_i128(5)),
                Some(i256::from_i128(-2)),
                None,
                Some(i256::from_i128(5)),
            ])
            .with_precision_and_scale(76, 0)
            .unwrap(),
        );
        let dictionary = Dictionary::build(&d256, &Cancellation::default())
            .unwrap()
            .expect("encoded");
        assert_eq!(dictionary.n_values(), 2);
        assert!((dictionary.null_count() > 0));
        assert_eq!(dictionary.code_at(0), dictionary.code_at(3));
        assert_ne!(dictionary.code_at(0), dictionary.code_at(1));
        assert!(dictionary.is_null(2));
    }
}

mod boolean {
    use arrow::array::BooleanArray;

    use super::*;

    #[test]
    fn boolean_arrays_group_distinct_values() {
        let array: ArrayRef = Arc::new(BooleanArray::from(vec![
            Some(true),
            Some(false),
            Some(true),
            None,
        ]));
        let dictionary = Dictionary::build(&array, &Cancellation::default())
            .unwrap()
            .expect("encoded");
        assert_eq!(dictionary.n_values(), 2);
        assert!((dictionary.null_count() > 0));
        assert_eq!(dictionary.code_at(0), dictionary.code_at(2));
        assert_ne!(dictionary.code_at(0), dictionary.code_at(1));
        assert!(dictionary.is_null(3));
    }
}
