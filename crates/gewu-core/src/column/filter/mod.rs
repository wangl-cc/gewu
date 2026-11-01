//! Column-level predicate compilation for explicit filters and Find queries.
//!
//! Type policy comes from the column's [`Type`] model: Find asks
//! [`Type::find_strategy`] and explicit filters are gated by
//! [`crate::types::ColumnCapabilities`]. The modules below own the native
//! kernels, not the decision of which types support which operation.

pub mod nested;
pub mod range;
pub mod scalar;
pub mod text;

use arrow::array::{Array, ArrayRef};
pub use range::Bound;

use crate::{
    predicate::Predicate,
    selection::RowSelection,
    types::{ColumnCapabilities, Domain, FindStrategy, Type},
    Error, Filter, Result,
};

/// Type-aware search for Find operations across all columns.
pub(crate) fn prepare_find<'a>(
    array: &'a ArrayRef,
    ty: &Type,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    if query.is_empty() {
        return Ok(None);
    }
    match ty.find_strategy() {
        FindStrategy::Unsupported => Ok(None),
        FindStrategy::Text => text::contains(array, query, false, matcher).map(Some),
        FindStrategy::Scalar => scalar::search(array, query),
        FindStrategy::List | FindStrategy::Struct => nested::search(array, query, matcher),
        FindStrategy::Dictionary => nested::dictionary_find(array, |values| {
            // The value type comes from the array itself so nested dictionaries
            // keep following keys level by level.
            let child = Type::from_arrow(values.data_type())
                .ok_or_else(|| Error::Unsupported("dictionary value type is not modeled".into()))?;
            prepare_find(values, &child, query, matcher)
        }),
    }
}

/// Type-dependent filters. InValues is prepared separately once the
/// column or document has obtained its lazy value index.
pub(crate) fn prepare_filter<'a>(
    array: &'a ArrayRef,
    ty: &Type,
    capabilities: ColumnCapabilities,
    filter: &Filter,
) -> Result<Predicate<'a>> {
    let predicate = prepare_value_filter(array, ty, capabilities, filter)?;
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

fn prepare_value_filter<'a>(
    array: &'a ArrayRef,
    ty: &Type,
    capabilities: ColumnCapabilities,
    filter: &Filter,
) -> Result<Predicate<'a>> {
    if let Filter::TextValues { values, exclude } = filter {
        return text::selected_values(array, values, *exclude);
    }
    if let Filter::IsNull { negate } = filter {
        return Ok(is_null_predicate(array, *negate));
    }
    if ty.is_dictionary() {
        return nested::dictionary(array, |values| {
            let child = Type::from_arrow(values.data_type())
                .ok_or_else(|| Error::Unsupported("dictionary value type is not modeled".into()))?;
            let child_caps = child.capabilities();
            prepare_filter(values, &child, child_caps, filter).map(Some)
        })?
        .ok_or_else(|| {
            Error::InvalidArgument("filter cannot be applied to dictionary values".into())
        });
    }
    match filter {
        Filter::Contains {
            text: value,
            case_sensitive,
            ..
        } if capabilities.contains => text::contains(array, value, *case_sensitive, None),
        Filter::Range { min, max, .. } if capabilities.range => {
            range::prepare_range(array, min.as_ref(), max.as_ref())
        }
        Filter::Equals { value } if capabilities.equals => {
            if is_text(ty) {
                Ok(text::equals(array, value))
            } else {
                scalar::equals(array, value)
            }
        }
        Filter::EqualsAny { values } if capabilities.equals => {
            let predicates = values
                .iter()
                .map(|value| {
                    if is_text(ty) {
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

/// Whether equality on this column compares strings rather than parsed values.
fn is_text(ty: &Type) -> bool {
    ty.domain() == Some(Domain::Text)
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
