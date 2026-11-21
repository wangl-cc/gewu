use std::sync::Arc;

use arrow::array::*;

use super::*;
use crate::{column::Column, Filter};

#[test]
fn physical_values_roundtrip_without_formatting_or_float_widening() {
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Float32Array::from(vec![0.1_f32, 0.1_f32.next_up()])),
        Arc::new(Float64Array::from(vec![
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE.next_up(),
        ])),
        Arc::new(
            Decimal256Array::from(vec![
                i256::MAX / i256::from(100),
                i256::MAX / i256::from(100) - i256::ONE,
            ])
            .with_precision_and_scale(76, 28)
            .unwrap(),
        ),
        Arc::new(Date64Array::from(vec![1_i64, 2])),
        Arc::new(Time64NanosecondArray::from(vec![1_i64, 2])),
        Arc::new(DurationNanosecondArray::from(vec![
            9_007_199_254_740_992_i64,
            9_007_199_254_740_993,
        ])),
        Arc::new(BooleanArray::from(vec![false, true])),
        Arc::new(StringArray::from(vec!["é", "e\u{301}"])),
    ];
    for array in arrays {
        let column = Column::new("value", array.clone()).unwrap();
        for row in 0..2 {
            let text = encode(&array, row).unwrap();
            let filter = Filter::exact(vec![text], false, false);
            let mut matched = [false; 2];
            column
                .compile_filter(&filter)
                .unwrap()
                .evaluate(0..2, &mut matched);
            assert_eq!(matched, [row == 0, row == 1], "{:?}", array.data_type());
        }
    }
}
