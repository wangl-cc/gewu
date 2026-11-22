use std::sync::Arc;

use arrow::{
    array::{BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, StringArray},
    compute::cast,
};

use super::*;
use crate::{
    execution::predicate::{Predicate, CHUNK},
    TextMatch,
};

/// Every leaf predicate a filter or Find can compile for `array`.
fn leaves(array: &ArrayRef) -> Vec<(String, Predicate<'_>)> {
    let data_type = array.data_type();
    let mut leaves = Vec::new();
    if range::supports(data_type) {
        for (lo, hi) in [("10", "60"), ("1.5", "40"), ("1970-01-11", "1970-03-01")] {
            let (Ok(lo), Ok(hi)) = (Bound::parse(data_type, lo), Bound::parse(data_type, hi))
            else {
                continue;
            };
            if let Ok(p) = range::compile_range(array, Some(&lo), Some(&hi)) {
                leaves.push((format!("range {lo:?}..{hi:?}"), p));
            }
        }
    }
    if range::supports(data_type) || *data_type == DataType::Boolean {
        for value in ["7", "NaN", "2.5", "true", "inf"] {
            if let Ok(p) = scalar::equals(array, value) {
                leaves.push((format!("equals {value}"), p));
            }
        }
    }
    if supports_text_match(data_type) {
        for (text, mode, case_sensitive) in [
            ("pp", TextMatch::Contains, false),
            ("-01", TextMatch::Contains, true),
            ("APPLE", TextMatch::Equals, false),
            ("Item-00042", TextMatch::Equals, true),
        ] {
            let p = text::text_match(array, text, mode, case_sensitive).unwrap();
            leaves.push((format!("{mode:?} {text} {case_sensitive}"), p));
        }
        let values = ["apple".to_owned(), "Item-00007".to_owned()];
        leaves.push(("in".into(), text::selected_values(array, &values).unwrap()));
    }
    for query in ["1", "pp", "2.5", "1970-02", "true", "item-0004"] {
        if let Ok(Some(p)) = compile_find(array, query, None) {
            leaves.push((format!("find {query}"), p));
        }
    }
    leaves
}

#[test]
fn native_bitmaps_agree_with_chunk_evaluation() {
    let n = CHUNK + 1_237;
    let valid = |i: usize| i % 7 != 3;
    let high_card: ArrayRef = Arc::new(StringArray::from_iter(
        (0..n).map(|i| valid(i).then(|| format!("Item-{:05}", i % 2_000))),
    ));
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int32Array::from_iter(
            (0..n).map(|i| valid(i).then_some((i % 500) as i32 - 100)),
        )),
        Arc::new(Float64Array::from_iter((0..n).map(|i| {
            valid(i).then_some(if i % 11 == 0 {
                f64::NAN
            } else {
                (i % 300) as f64 / 4.0
            })
        }))),
        Arc::new(
            Decimal128Array::from_iter((0..n).map(|i| valid(i).then_some((i % 900) as i128)))
                .with_precision_and_scale(10, 2)
                .unwrap(),
        ),
        Arc::new(Date32Array::from_iter(
            (0..n).map(|i| valid(i).then_some((i % 400) as i32)),
        )),
        Arc::new(BooleanArray::from_iter(
            (0..n).map(|i| valid(i).then_some(i % 3 == 0)),
        )),
        Arc::new(StringArray::from_iter((0..n).map(|i| {
            valid(i).then(|| ["Apple", "apple", "pear", "Kiwi"][i % 4])
        }))),
        cast(&high_card, &DataType::Utf8View).unwrap(),
        high_card,
    ];
    let mut checked = 0;
    for array in &arrays {
        for (name, leaf) in leaves(array) {
            for rows in [0..n, 3..n - 5, CHUNK - 1..CHUNK + 70] {
                assert_eq!(
                    leaf.select(rows.clone()),
                    leaf.pack_chunks(rows.clone()),
                    "{} {name} {rows:?}",
                    array.data_type()
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 40, "{checked}");
}
