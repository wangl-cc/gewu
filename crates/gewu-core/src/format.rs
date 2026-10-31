//! Turning cell values into display text. The app and UniFFI share this so
//! every consumer shows exactly the same strings.

use arrow::{
    array::Array,
    util::display::{ArrayFormatter, FormatOptions},
};

use crate::Result;

pub(crate) fn options() -> FormatOptions<'static> {
    FormatOptions::default().with_display_error(true)
}

pub(crate) fn formatter(array: &dyn Array) -> Result<ArrayFormatter<'_>> {
    Ok(ArrayFormatter::try_new(array, &options())?)
}

/// Format one cell, `None` for null.
pub(crate) fn cell(array: &dyn Array, f: &ArrayFormatter<'_>, row: usize) -> Option<String> {
    if array.is_null(row) {
        None
    } else {
        Some(f.value(row).to_string())
    }
}
