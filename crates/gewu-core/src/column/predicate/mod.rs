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
    let (condition, include_nulls) = match filter {
        Filter::OnlyNulls => return Ok(null_selector(&column.array, true)),
        Filter::NotNull => return Ok(null_selector(&column.array, false)),
        Filter::Condition {
            condition,
            include_nulls,
        } => (condition, *include_nulls),
    };
    // Conditions that depend only on the value are evaluated once per distinct
    // dictionary value, then rows are matched by code.
    let data_type = column.array.data_type();
    let per_value = match condition {
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
            Some((condition.clone(), false))
        }
        _ => None,
    };
    if let Some((condition, exclude)) = per_value {
        if let Some(dictionary) = column.dictionary()? {
            let values = dictionary.values();
            let matched = compile_condition(values, &condition)?.select(0..values.len());
            return Ok(dictionary.matching_codes(&matched, exclude, include_nulls));
        }
    }
    let array = &column.array;
    let condition = compile_condition(array, condition)?;
    let nulls = array.logical_nulls();
    Ok(Selector::new(Box::new(move |rows| {
        with_nulls(
            condition.select(rows.clone()),
            nulls.as_ref(),
            &rows,
            include_nulls,
        )
    })))
}

/// Rows whose value is null, or with `nulls == false`, not null.
fn null_selector(array: &ArrayRef, nulls: bool) -> Selector<'static> {
    let mask = array.logical_nulls();
    Selector::new(Box::new(move |rows| {
        let bits = if nulls {
            arrow::buffer::BooleanBuffer::new_unset(rows.len())
        } else {
            arrow::buffer::BooleanBuffer::new_set(rows.len())
        };
        with_nulls(bits, mask.as_ref(), &rows, nulls)
    }))
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
mod tests;
