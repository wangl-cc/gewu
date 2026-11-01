//! Integer queries and prepared endpoints over native integer arrays.

use arrow::array::{downcast_integer_array, ArrayRef};

use super::decimal::{native_range, Endpoint, Literal};
use crate::predicate::Predicate;

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    let value = Literal::parse_integer(text)?.coefficient(0, true);
    Some(bind(array, value, value))
}

pub(in crate::column) fn bind(array: &ArrayRef, lo: Endpoint, hi: Endpoint) -> Predicate<'_> {
    downcast_integer_array!(array => native_range(array, lo, hi), _ => unreachable!("integer dispatch"))
}
