//! Column-level predicate compilation for explicit filters and Find queries.
//!
//! This module owns filter support rules and native predicate compilation.
//! Find compiles its own text queries in the private `find` module.

mod find;
pub(crate) use find::compile as compile_find;

pub mod nested;
pub mod range;
pub mod scalar;
pub mod text;

use arrow::{
    array::{downcast_dictionary_array, Array, ArrayRef},
    datatypes::{ArrowNativeType, DataType},
};

use crate::{
    execution::predicate::{collect_bits, with_nulls, Selector},
    scalar::Bound,
    types, Filter,
};

pub(crate) type Result<T> = std::result::Result<T, crate::FilterError>;

/// Text matching is defined only for text values, including dictionaries.
pub(crate) fn supports_text_match(data_type: &DataType) -> bool {
    matches!(
        types::value_type(data_type),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

/// Equality parses scalar values or compares text; dictionaries follow values.
pub(crate) fn supports_equals(data_type: &DataType) -> bool {
    let data_type = types::value_type(data_type);
    range::supports(data_type)
        || matches!(
            data_type,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean
        )
}

/// One column-bound compilation entry point, including dictionary acceleration
/// and the orthogonal null policy. No UI or cache representation enters Filter.
pub(crate) fn compile_filter<'a>(
    column: &'a super::Column,
    filter: &Filter,
) -> Result<Selector<'a>> {
    use crate::FilterCondition;
    // Conditions that depend only on the value are evaluated once per distinct
    // dictionary value, then rows are matched by code.
    let data_type = column.array.data_type();
    let per_value = match &filter.condition {
        FilterCondition::Exact { values, exclude }
            if !values.is_empty() && supports_equals(data_type) =>
        {
            let condition = FilterCondition::Exact {
                values: values.clone(),
                exclude: false,
            };
            Some((condition, *exclude))
        }
        FilterCondition::Text { .. } if supports_text_match(data_type) => {
            Some((filter.condition.clone(), false))
        }
        _ => None,
    };
    if let Some((condition, exclude)) = per_value {
        if let Some(dictionary) = column.dictionary()? {
            let values = dictionary.values();
            let matched = compile_condition(values, &condition)?.select(0..values.len());
            return Ok(dictionary.matching_codes(&matched, exclude, filter.include_nulls));
        }
    }
    let array = &column.array;
    let condition = compile_condition(array, &filter.condition)?;
    let nulls = array.logical_nulls();
    let include_nulls = filter.include_nulls;
    Ok(Selector::new(Box::new(move |rows| {
        with_nulls(
            condition.select(rows.clone()),
            nulls.as_ref(),
            &rows,
            include_nulls,
        )
    })))
}

/// Raw value matches of `condition`; null rows are unselected.
fn compile_condition<'a>(
    array: &'a ArrayRef,
    condition: &crate::FilterCondition,
) -> Result<Selector<'a>> {
    use crate::FilterCondition;
    if let FilterCondition::Exact { values, exclude } = condition {
        if values.is_empty() {
            return Ok(Selector::constant(*exclude));
        }
    }
    if matches!(array.data_type(), DataType::Dictionary(..)) {
        return dictionary_condition(array, condition);
    }
    Ok(match condition {
        FilterCondition::Text {
            text: value,
            mode,
            case_sensitive,
        } if supports_text_match(array.data_type()) => {
            text::text_match(array, value, *mode, *case_sensitive)?.into()
        }
        FilterCondition::Range { min, max } if range::supports(array.data_type()) => {
            let min = min
                .as_deref()
                .map(|text| Bound::parse(array.data_type(), text))
                .transpose()?;
            let max = max
                .as_deref()
                .map(|text| Bound::parse(array.data_type(), text))
                .transpose()?;
            range::compile_range(array, min.as_ref(), max.as_ref())?.into()
        }
        FilterCondition::Exact { values, exclude } if supports_equals(array.data_type()) => {
            let selector: Selector<'a> = if matches!(
                types::value_type(array.data_type()),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            ) {
                text::selected_values(array, values)?.into()
            } else {
                Selector::any_of(
                    values
                        .iter()
                        .map(|value| scalar::equals(array, value).map(Selector::from))
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            if *exclude {
                selector.negate()
            } else {
                selector
            }
        }
        _ => {
            return Err(crate::FilterError::UnsupportedType {
                condition: condition.into(),
                data_type: array.data_type().clone(),
            })
        }
    })
}

/// Evaluate `condition` once per dictionary value, then select rows by key.
fn dictionary_condition<'a>(
    array: &'a ArrayRef,
    condition: &crate::FilterCondition,
) -> Result<Selector<'a>> {
    downcast_dictionary_array!(array => {
        let values = array.values();
        let matched = compile_condition(values, condition)?.select(0..values.len());
        let keys = array.keys();
        Ok(Selector::new(Box::new(move |rows| {
            collect_bits(rows, |row| keys.is_valid(row) && matched.value(keys.value(row).as_usize()))
        })))
    }, _ => unreachable!("dictionary type checked by caller"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, StringArray,
        },
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
}
