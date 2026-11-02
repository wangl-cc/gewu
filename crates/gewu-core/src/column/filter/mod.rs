//! Column-level predicate compilation for explicit filters and Find queries.
//!
//! This module owns filter support rules and native predicate preparation.
//! Find compiles its own text queries in the private `find` module.

mod find;
pub(crate) use find::prepare as prepare_find;

pub mod nested;
pub mod range;
pub mod scalar;
pub mod text;

use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
};

pub use super::bound::Bound;
use crate::{
    predicate::Predicate,
    types::{self, Domain, Encoding},
    Error, Filter, Result,
};

/// Types with a useful displayed-text or element-containment filter.
pub(crate) fn supports_contains(data_type: &DataType) -> bool {
    let data_type = types::value_type(data_type);
    matches!(
        types::direct_domain(data_type),
        Some(Domain::Text | Domain::Binary | Domain::Interval)
    ) || matches!(
        data_type,
        DataType::Null
            | DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::Map(..)
    )
}

/// Equality parses scalar values or compares text; dictionaries follow values.
pub(crate) fn supports_equals(data_type: &DataType) -> bool {
    let data_type = types::value_type(data_type);
    range::supports(data_type)
        || matches!(
            types::direct_domain(data_type),
            Some(Domain::Text | Domain::Boolean)
        )
}

/// One column-bound compilation entry point, including dictionary acceleration
/// and the orthogonal null policy. No UI or cache representation enters Filter.
pub(crate) fn prepare_filter<'a>(
    column: &'a super::Column,
    filter: &Filter,
) -> Result<Predicate<'a>> {
    use crate::FilterCondition;
    if let FilterCondition::Exact { values, exclude } = &filter.condition {
        if !values.is_empty() && supports_equals(column.array.data_type()) {
            if let Some(dictionary) = column.dictionary() {
                let selection = prepare_condition(dictionary.values(), &FilterCondition::Exact {
                    values: values.clone(),
                    exclude: false,
                })?;
                let mut matched = vec![false; dictionary.values().len()];
                selection.evaluate(
                    0..matched.len(),
                    &mut matched,
                    &crate::engine::CancelToken::never(),
                )?;
                return Ok(dictionary.matching_codes(&matched, *exclude, filter.include_nulls));
            }
        }
    }
    let array = &column.array;
    let predicate = prepare_condition(array, &filter.condition)?;
    let nulls = array.logical_nulls();
    let include_nulls = filter.include_nulls;
    Ok(Predicate::new(Box::new(
        move |rows, incoming, output, cancel| {
            predicate.evaluate_chunk(rows.clone(), incoming, output, cancel)?;
            for (i, row) in rows.iter().enumerate() {
                if nulls.as_ref().is_some_and(|nulls| nulls.is_null(row)) {
                    output[i] = include_nulls && incoming.is_none_or(|mask| mask[i]);
                }
            }
            Ok(())
        },
    )))
}

fn prepare_condition<'a>(
    array: &'a ArrayRef,
    condition: &crate::FilterCondition,
) -> Result<Predicate<'a>> {
    use crate::FilterCondition;
    if let FilterCondition::Exact { values, exclude } = condition {
        if values.is_empty() {
            let all = *exclude;
            return Ok(Predicate::new(Box::new(
                move |_rows, incoming, output, _cancel| {
                    for (i, value) in output.iter_mut().enumerate() {
                        *value = all && incoming.is_none_or(|mask| mask[i]);
                    }
                    Ok(())
                },
            )));
        }
    }
    if types::encoding(array.data_type()) == Encoding::Dictionary {
        return nested::dictionary(array, |values| {
            prepare_condition(values, condition).map(Some)
        })?
        .ok_or_else(|| {
            Error::InvalidArgument("filter cannot be applied to dictionary values".into())
        });
    }
    match condition {
        FilterCondition::Contains {
            text: value,
            case_sensitive,
        } if supports_contains(array.data_type()) => {
            text::contains(array, value, *case_sensitive, None)
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
            range::prepare_range(array, min.as_ref(), max.as_ref())
        }
        FilterCondition::Exact { values, exclude } if supports_equals(array.data_type()) => {
            let predicate = if types::domain(array.data_type()) == Some(Domain::Text) {
                text::selected_values(array, values)?
            } else {
                Predicate::any_of(
                    values
                        .iter()
                        .map(|value| scalar::equals(array, value))
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            if !exclude {
                return Ok(predicate);
            }
            Ok(Predicate::new(Box::new(
                move |rows, incoming, output, cancel| {
                    predicate.evaluate_chunk(rows, incoming, output, cancel)?;
                    for (i, value) in output.iter_mut().enumerate() {
                        *value = !*value && incoming.is_none_or(|mask| mask[i]);
                    }
                    Ok(())
                },
            )))
        }
        _ => Err(Error::InvalidArgument(format!(
            "filter is not supported for {}",
            array.data_type()
        ))),
    }
}
