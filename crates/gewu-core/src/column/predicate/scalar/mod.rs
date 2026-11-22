//! Scalar type semantics. Compilation borrows the native array and parses
//! inputs once; evaluating a predicate is the executor's responsibility.

pub(crate) mod decimal;
pub(crate) mod float;
pub(crate) mod integer;
pub(crate) mod temporal;

use arrow::array::{Array, ArrayRef, AsArray};

use super::range::compile_range;
use crate::{column::predicate::Result, execution::predicate::Predicate, scalar::Bound};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Result<Option<Predicate<'a>>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    use arrow::datatypes::DataType::*;
    Ok(match array.data_type() {
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            integer::search(array, text)
        }
        Float16 | Float32 | Float64 => float::search(array, text),
        Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..) => {
            decimal::search(array, text)
        }
        Boolean => boolean(text).map(|target| bool_predicate(array, target)),
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) | Duration(_) => {
            return temporal::search(array, text);
        }
        _ => {
            return Err(crate::InternalError::Invariant(format!(
                "{} reached scalar Find dispatch without scalar semantics",
                array.data_type()
            ))
            .into());
        }
    })
}

pub(super) fn equals<'a>(array: &'a ArrayRef, text: &str) -> Result<Predicate<'a>> {
    let text = text.trim();
    match array.data_type() {
        arrow::datatypes::DataType::Boolean => boolean(text)
            .map(|target| bool_predicate(array, target))
            .ok_or_else(|| crate::FilterError::InvalidValue {
                value: text.into(),
                expected: arrow::datatypes::DataType::Boolean,
                reason: crate::ValueParseError::InvalidSyntax,
            }),
        arrow::datatypes::DataType::Float16
        | arrow::datatypes::DataType::Float32
        | arrow::datatypes::DataType::Float64 => float::equals(array, text),
        _ => {
            let bound = Bound::parse(array.data_type(), text)?;
            compile_range(array, Some(&bound), Some(&bound))
        }
    }
}

fn boolean(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn bool_predicate(array: &ArrayRef, target: bool) -> Predicate<'_> {
    let array = array.as_boolean();
    let has_nulls = array.null_count() > 0;
    Predicate::new(Box::new(move |rows, output| {
        for (i, row) in rows.iter().enumerate() {
            output[i] = (!has_nulls || array.is_valid(row)) && array.value(row) == target;
        }
    }))
}

#[cfg(test)]
mod tests;
