//! Shared formatting of cell values for display, search and text export.

use arrow::{
    array::Array,
    util::display::{ArrayFormatter, FormatOptions},
};

const FORMAT_OPTIONS: FormatOptions<'static> = FormatOptions::new().with_display_error(true);

pub(crate) fn formatter(
    array: &dyn Array,
) -> std::result::Result<ArrayFormatter<'_>, arrow::error::ArrowError> {
    ArrayFormatter::try_new(array, &FORMAT_OPTIONS)
}

/// Format one cell, `None` for null.
pub(crate) fn cell(array: &dyn Array, f: &ArrayFormatter<'_>, row: usize) -> Option<String> {
    if crate::column::is_null_at(array, row) {
        None
    } else {
        Some(f.value(row).to_string())
    }
}
