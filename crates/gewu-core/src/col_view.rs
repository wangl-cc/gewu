//! A column and the source rows selected by an immutable view.

use std::ops::Range;

use arrow::{array::ArrayRef, buffer::BooleanBuffer, datatypes::DataType};

use crate::{
    column::Column,
    row_ids::{RowSlice, RowWindow},
};

#[derive(Clone, Copy)]
pub(crate) struct ColView<'a> {
    column: &'a Column,
    rows: RowSlice<'a>,
    /// Filter bitmap selecting exactly the rows of `rows`, when one exists.
    mask: Option<&'a BooleanBuffer>,
}

impl<'a> ColView<'a> {
    pub(crate) fn new(column: &'a Column, rows: RowSlice<'a>) -> Self {
        Self {
            column,
            rows,
            mask: None,
        }
    }

    /// Attach the filter bitmap whose set bits are exactly this view's rows,
    /// so order-independent scans can walk the bitmap instead of row IDs.
    pub(crate) fn with_mask(mut self, mask: Option<&'a BooleanBuffer>) -> Self {
        self.mask = mask;
        self
    }

    pub(crate) fn column(&self) -> &Column {
        self.column
    }

    pub(crate) fn array(&self) -> &ArrayRef {
        self.column.array()
    }

    pub(crate) fn rows(&self) -> RowSlice<'a> {
        self.rows
    }

    pub(crate) fn data_type(&self) -> &DataType {
        self.array().data_type()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_complete(&self) -> bool {
        matches!(self.rows, RowSlice::All(_)) || self.rows.len() == self.array().len()
    }

    /// Source rows for order-independent scans. A complete view is the
    /// contiguous range even when its display rows are sorted; a filtered
    /// view scans its filter bitmap in source order.
    pub(crate) fn selection(&self) -> Selection<'a> {
        if self.is_complete() {
            Selection::Range(0..self.array().len())
        } else if let Some(mask) = self.mask {
            Selection::Mask(mask)
        } else {
            Selection::from(self.rows)
        }
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

impl<'a> From<RowSlice<'a>> for Selection<'a> {
    fn from(rows: RowSlice<'a>) -> Self {
        match rows {
            RowSlice::All(n) => Self::Range(0..n),
            RowSlice::Selected(rows) => Self::Rows(rows),
        }
    }
}

#[cfg(test)]
pub(crate) mod test {
    use std::sync::Arc;

    use super::*;
    use crate::row_ids::RowIds;

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
            ColView::new(&self.column, self.rows.as_slice()).with_mask(self.mask.as_ref())
        }
    }
}
