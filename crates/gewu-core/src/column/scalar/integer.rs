//! Integer queries and exact raw-text bounds over native integer arrays.

use arrow::array::{downcast_integer_array, ArrayRef};

use super::decimal::{endpoints, native_range, Endpoint, Literal};
use crate::{predicate::Predicate, Result};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    // Search means an integer literal, not a precision interval. Parsing the
    // value as digits also admits out-of-domain queries without rounding them.
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Literal::parse(text)?;
    range(array, Some(text), Some(text)).ok()
}

pub(super) fn range<'a>(
    array: &'a ArrayRef,
    min: Option<&str>,
    max: Option<&str>,
) -> Result<Predicate<'a>> {
    let (lo, hi) = endpoints(min, max, 0)?;
    Ok(bind(array, lo, hi))
}

pub(in crate::column) fn bind(array: &ArrayRef, lo: Endpoint, hi: Endpoint) -> Predicate<'_> {
    downcast_integer_array!(array => native_range(array, lo, hi), _ => unreachable!("integer dispatch"))
}
