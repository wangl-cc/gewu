//! Columnar scans over a column and a [`Selection`].
//!
//! Each helper matches the selection and the presence of nulls once, then
//! runs a monomorphic loop: contiguous ranges read the native values slice
//! (and walk set validity bits when nulls exist); explicit rows index the
//! slice directly; bitmaps are ANDed with the validity bitmap first. Callbacks receive source rows,
//! so results can address the original column without materializing a copy.

use arrow::{
    array::{Array, PrimitiveArray},
    buffer::NullBuffer,
    datatypes::ArrowPrimitiveType,
};

use crate::col_view::Selection;

/// Visit `(row, value)` for every valid value in scan order.
#[inline]
pub(crate) fn for_each_valid<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    mut f: impl FnMut(usize, T::Native),
) {
    let values = array.values();
    for_each_valid_row(array.nulls(), selection, |row| f(row, values[row]));
}

/// Visit every row of `selection` that `nulls` marks valid, in scan order.
#[inline]
pub(crate) fn for_each_valid_row(
    nulls: Option<&NullBuffer>,
    selection: &Selection<'_>,
    mut f: impl FnMut(usize),
) {
    match (selection, nulls.filter(|nulls| nulls.null_count() > 0)) {
        (Selection::Range(range), None) => range.clone().for_each(f),
        (Selection::Range(range), Some(nulls)) => {
            let bits = nulls.inner().slice(range.start, range.len());
            for offset in bits.set_indices() {
                f(range.start + offset);
            }
        }
        (Selection::Rows(rows), None) => {
            for &row in *rows {
                f(row as usize);
            }
        }
        (Selection::Rows(rows), Some(nulls)) => {
            for &row in *rows {
                let row = row as usize;
                if nulls.is_valid(row) {
                    f(row);
                }
            }
        }
        (Selection::Mask(mask), None) => mask.set_indices().for_each(f),
        (Selection::Mask(mask), Some(nulls)) => (*mask & nulls.inner()).set_indices().for_each(f),
    }
}

/// Number of rows of `selection` that `nulls` marks null.
pub(crate) fn null_count(nulls: Option<&NullBuffer>, selection: &Selection<'_>) -> u64 {
    let Some(nulls) = nulls.filter(|nulls| nulls.null_count() > 0) else {
        return 0;
    };
    match selection {
        Selection::Range(range) if range.len() == nulls.len() => nulls.null_count() as u64,
        Selection::Range(range) => {
            (range.len()
                - nulls
                    .inner()
                    .slice(range.start, range.len())
                    .count_set_bits()) as u64
        }
        Selection::Rows(rows) => rows
            .iter()
            .map(|&row| u64::from(nulls.is_null(row as usize)))
            .sum(),
        Selection::Mask(mask) => {
            (mask.count_set_bits() - (*mask & nulls.inner()).count_set_bits()) as u64
        }
    }
}

/// [`for_each_valid_row`] over an array's own validity.
#[inline]
pub(crate) fn for_each_valid_index(
    array: &dyn Array,
    selection: &Selection<'_>,
    f: impl FnMut(usize),
) {
    for_each_valid_row(array.nulls(), selection, f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_counts_and_valid_rows_follow_each_selection_kind() {
        let nulls = NullBuffer::from(vec![true, false, true, false, true]);
        let mask = arrow::buffer::BooleanBuffer::from(vec![false, true, true, true, false]);
        let selections = [
            Selection::Range(0..5),
            Selection::Range(1..4),
            Selection::Rows(&[4, 3, 1]),
            Selection::Mask(&mask),
        ];
        for (selection, nulls_seen, valid) in [
            (&selections[0], 2, vec![0, 2, 4]),
            (&selections[1], 2, vec![2]),
            (&selections[2], 2, vec![4]),
            (&selections[3], 2, vec![2]),
        ] {
            assert_eq!(
                null_count(Some(&nulls), selection),
                nulls_seen,
                "{selection:?}"
            );
            let mut rows = Vec::new();
            for_each_valid_row(Some(&nulls), selection, |row| rows.push(row));
            assert_eq!(rows, valid, "{selection:?}");
            let mut all = Vec::new();
            for_each_valid_row(None, selection, |row| all.push(row));
            assert_eq!(all.len(), selection.len(), "{selection:?}");
            assert_eq!(null_count(None, selection), 0);
        }
    }
}
