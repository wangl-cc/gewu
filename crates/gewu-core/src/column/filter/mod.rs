//! Column-level predicate compilation for explicit filters and Find queries.

pub mod nested;
pub mod range;
pub mod scalar;
pub mod text;

use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
};
pub use range::Bound;

use crate::{
    column::kind::{ColumnCapabilities, LogicalType},
    filter::Filter,
    predicate::Predicate,
    selection::RowSelection,
    Error, Result,
};

/// Type-aware search for Find operations across all columns.
pub(crate) fn prepare_find<'a>(
    array: &'a ArrayRef,
    logical_type: &LogicalType,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    if query.is_empty() {
        return Ok(None);
    }
    if matches!(array.data_type(), DataType::Dictionary(..)) {
        return nested::dictionary_find(array, |values| {
            let child_logical = LogicalType::from_arrow(values.data_type());
            prepare_find(values, &child_logical, query, matcher)
        });
    }
    match logical_type {
        LogicalType::Text => text::contains(array, query, false, matcher).map(Some),
        LogicalType::Integer { .. }
        | LogicalType::Float { .. }
        | LogicalType::Decimal { .. }
        | LogicalType::Boolean
        | LogicalType::Date
        | LogicalType::Time
        | LogicalType::Timestamp { .. }
        | LogicalType::Duration { .. } => scalar::search(array, query),
        LogicalType::List | LogicalType::Struct => nested::search(array, query, matcher),
        LogicalType::Binary | LogicalType::Interval | LogicalType::Other { .. } => Ok(None),
    }
}

/// Type-dependent filters. InValues is prepared separately once the
/// column or document has obtained its lazy value index.
pub(crate) fn prepare_filter<'a>(
    array: &'a ArrayRef,
    logical_type: &LogicalType,
    capabilities: ColumnCapabilities,
    filter: &Filter,
) -> Result<Predicate<'a>> {
    let predicate = prepare_value_filter(array, logical_type, capabilities, filter)?;
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
    logical_type: &LogicalType,
    capabilities: ColumnCapabilities,
    filter: &Filter,
) -> Result<Predicate<'a>> {
    if let Filter::TextValues { values, exclude } = filter {
        return text::selected_values(array, values, *exclude);
    }
    if let Filter::IsNull { negate } = filter {
        return Ok(is_null_predicate(array, *negate));
    }
    if matches!(array.data_type(), DataType::Dictionary(..)) {
        return nested::dictionary(array, |values| {
            let child_logical = LogicalType::from_arrow(values.data_type());
            let child_caps =
                child_logical.capabilities(matches!(values.data_type(), DataType::Dictionary(..)));
            prepare_filter(values, &child_logical, child_caps, filter).map(Some)
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
            if *logical_type == LogicalType::Text {
                Ok(text::equals(array, value))
            } else {
                scalar::equals(array, value)
            }
        }
        Filter::EqualsAny { values } if capabilities.equals => {
            let predicates = values
                .iter()
                .map(|value| {
                    if *logical_type == LogicalType::Text {
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
