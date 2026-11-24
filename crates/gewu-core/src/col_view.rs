//! A column and the source rows selected by an immutable view.

use std::ops::Range;

use arrow::{array::ArrayRef, buffer::BooleanBuffer, datatypes::DataType};

use crate::{column::Column, row_ids::RowWindow};

pub(crate) struct ColView<'a> {
    column: &'a Column,
    selection: Selection<'a>,
}

impl<'a> ColView<'a> {
    pub(crate) fn from_mask(column: &'a Column, mask: Option<&'a BooleanBuffer>) -> Self {
        let selection = match mask {
            Some(mask) => Selection::Mask(mask),
            None => Selection::Range(0..column.array().len()),
        };
        Self { column, selection }
    }

    pub(crate) fn column(&self) -> &Column {
        self.column
    }

    pub(crate) fn array(&self) -> &ArrayRef {
        self.column.array()
    }

    pub(crate) fn data_type(&self) -> &DataType {
        self.array().data_type()
    }

    pub(crate) fn selection(&self) -> &Selection<'a> {
        &self.selection
    }
}

/// The source rows a columnar scan reads, independent of display order.
///
/// Analyses take a column plus a `Selection` and dispatch on the variant once
/// per scan (see `stats::scan`), so each inner loop is monomorphic: a
/// contiguous range reads native slices and the validity bitmap directly,
/// explicit rows gather by index, and a bitmap is combined with the validity
/// bitmap word by word before its set bits are visited.
#[derive(Clone, Debug)]
pub(crate) enum Selection<'a> {
    /// Contiguous source rows.
    Range(Range<usize>),
    /// Explicit source rows, scanned in this order.
    Rows(&'a [u32]),
    /// Set bits over every source row, scanned in source order.
    Mask(&'a BooleanBuffer),
}

impl Selection<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Range(range) => range.len(),
            Self::Rows(rows) => rows.len(),
            Self::Mask(mask) => mask.count_set_bits(),
        }
    }
}

impl<'a> From<RowWindow<'a>> for Selection<'a> {
    fn from(rows: RowWindow<'a>) -> Self {
        match rows {
            RowWindow::Range { start, end } => Self::Range(start..end),
            RowWindow::Slice(rows) => Self::Rows(rows),
        }
    }
}

#[cfg(test)]
pub(crate) mod test {
    use std::sync::Arc;

    use super::*;
    use crate::row_ids::{RowIds, RowSlice};

    pub(crate) struct Fixture {
        column: Column,
        rows: RowIds,
        mask: Option<BooleanBuffer>,
    }

    impl Fixture {
        pub(crate) fn all(array: &ArrayRef) -> Self {
            Self {
                column: Column::new("test", array.clone()).unwrap(),
                rows: RowIds::All(array.len()),
                mask: None,
            }
        }

        pub(crate) fn new(array: &ArrayRef, rows: Vec<u32>) -> Self {
            Self {
                column: Column::new("test", array.clone()).unwrap(),
                rows: RowIds::Selected(Arc::from(rows)),
                mask: None,
            }
        }

        /// A filtered view of `rows` (ascending), carrying its filter bitmap.
        pub(crate) fn masked(array: &ArrayRef, rows: Vec<u32>) -> Self {
            let mask = BooleanBuffer::collect_bool(array.len(), |row| {
                rows.binary_search(&(row as u32)).is_ok()
            });
            Self {
                mask: Some(mask),
                ..Self::new(array, rows)
            }
        }

        pub(crate) fn view(&self) -> ColView<'_> {
            match (&self.mask, self.rows.as_slice()) {
                (None, RowSlice::Selected(rows)) => ColView {
                    column: &self.column,
                    selection: Selection::Rows(rows),
                },
                (mask, _) => ColView::from_mask(&self.column, mask.as_ref()),
            }
        }

        pub(crate) fn rows(&self) -> RowWindow<'_> {
            self.rows.as_slice().window(0, self.rows.len())
        }
    }
}
