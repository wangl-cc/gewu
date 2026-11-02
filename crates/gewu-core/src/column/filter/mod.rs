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
    selection::RowSelection,
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

/// Type-dependent filters. InValues is prepared separately once the
/// column or document has obtained its lazy value index.
pub(crate) fn prepare_filter<'a>(array: &'a ArrayRef, filter: &Filter) -> Result<Predicate<'a>> {
    let predicate = prepare_value_filter(array, filter)?;
    let include_nulls = matches!(
        filter,
        Filter::Range {
            include_nulls: true,
            ..
        } | Filter::Contains {
            include_nulls: true,
            ..
        }
    );
    if !include_nulls || array.null_count() == 0 {
        return Ok(predicate);
    }
    Ok(Predicate::new(Box::new(
        move |rows, incoming, output, cancel| {
            predicate.evaluate_chunk(rows.clone(), incoming, output, cancel)?;
            match incoming {
                Some(mask) => {
                    for (index, row) in rows.iter().enumerate() {
                        output[index] |= array.is_null(row) & mask[index];
                    }
                }
                None => {
                    for (index, row) in rows.iter().enumerate() {
                        output[index] |= array.is_null(row);
                    }
                }
            }
            Ok(())
        },
    )))
}

fn prepare_value_filter<'a>(array: &'a ArrayRef, filter: &Filter) -> Result<Predicate<'a>> {
    if let Filter::TextValues { values, exclude } = filter {
        return text::selected_values(array, values, *exclude);
    }
    if let Filter::IsNull { negate } = filter {
        return Ok(is_null_predicate(array, *negate));
    }
    if types::encoding(array.data_type()) == Encoding::Dictionary {
        return nested::dictionary(array, |values| prepare_filter(values, filter).map(Some))?
            .ok_or_else(|| {
                Error::InvalidArgument("filter cannot be applied to dictionary values".into())
            });
    }
    match filter {
        Filter::Contains {
            text: value,
            case_sensitive,
            ..
        } if supports_contains(array.data_type()) => {
            text::contains(array, value, *case_sensitive, None)
        }
        Filter::Range { min, max, .. } if range::supports(array.data_type()) => {
            range::prepare_range(array, min.as_ref(), max.as_ref())
        }
        Filter::Equals { value } if supports_equals(array.data_type()) => {
            if types::domain(array.data_type()) == Some(Domain::Text) {
                Ok(text::equals(array, value))
            } else {
                scalar::equals(array, value)
            }
        }
        Filter::EqualsAny { values } if supports_equals(array.data_type()) => {
            let predicates = values
                .iter()
                .map(|value| {
                    if types::domain(array.data_type()) == Some(Domain::Text) {
                        Ok(text::equals(array, value))
                    } else {
                        scalar::equals(array, value)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Predicate::any_of(predicates))
        }
        _ => Err(Error::InvalidArgument(format!(
            "filter is not supported for {}",
            array.data_type()
        ))),
    }
}

fn is_null_predicate<'a>(array: &'a ArrayRef, negate: bool) -> Predicate<'a> {
    Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                for (i, row) in range.enumerate() {
                    output[i] = array.is_null(row) != negate;
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                for (i, row) in range.enumerate() {
                    output[i] = (array.is_null(row) != negate) & mask[i];
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = array.is_null(row) != negate;
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = (array.is_null(row) != negate) & mask[i];
                }
            }
        }
        Ok(())
    }))
}
