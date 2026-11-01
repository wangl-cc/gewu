//! Turning cell values into display text. The app and UniFFI share this so
//! every consumer shows exactly the same strings.

use arrow::{
    array::Array,
    util::display::{ArrayFormatter, FormatOptions},
};

use crate::Result;

const FORMAT_OPTIONS: FormatOptions<'static> = FormatOptions::new().with_display_error(true);

pub(crate) fn formatter(array: &dyn Array) -> Result<ArrayFormatter<'_>> {
    Ok(ArrayFormatter::try_new(array, &FORMAT_OPTIONS)?)
}

/// Format one cell, `None` for null.
pub(crate) fn cell(array: &dyn Array, f: &ArrayFormatter<'_>, row: usize) -> Option<String> {
    if array.is_null(row) {
        None
    } else {
        Some(f.value(row).to_string())
    }
}
