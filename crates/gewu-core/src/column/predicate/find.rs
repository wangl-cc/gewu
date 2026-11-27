//! Find predicate compilation. Type dispatch and text parsing stay inside
//! this operation; callers provide arrays and consume compiled predicates.

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{nested, scalar, text, Result};
use crate::execution::predicate::Predicate;

pub(crate) fn compile<'a>(
    array: &'a ArrayRef,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    if query.is_empty() {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Dictionary(..) => {
            nested::dictionary_find(array, |values| compile(values, query, matcher))
        }
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::FixedSizeList(..)
        | DataType::Struct(_) => nested::search(array, query, matcher),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            text::contains(array, query, false, matcher).map(Some)
        }
        DataType::Boolean
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Decimal32(..)
        | DataType::Decimal64(..)
        | DataType::Decimal128(..)
        | DataType::Decimal256(..)
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(..)
        | DataType::Duration(_) => scalar::search(array, query),
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
