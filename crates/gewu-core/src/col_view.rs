//! A column and the source rows selected by an immutable view.

use std::ops::Range;

use arrow::{array::ArrayRef, datatypes::DataType};

use crate::{column::Column, row_ids::RowSlice};

#[derive(Clone, Copy)]
pub(crate) struct ColView<'a> {
    column: &'a Column,
    rows: RowSlice<'a>,
}

impl<'a> ColView<'a> {
    pub(crate) fn new(column: &'a Column, rows: RowSlice<'a>) -> Self {
        Self { column, rows }
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
    /// contiguous range even when its display rows are sorted.
    pub(crate) fn selection(&self) -> Selection<'a> {
        if self.is_complete() {
            Selection::Range(0..self.array().len())
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
/// explicit rows gather by index. A bitmap selection can be added as another
/// variant without changing the analyses.
#[derive(Clone, Debug)]
pub(crate) enum Selection<'a> {
    /// Contiguous source rows.
    Range(Range<usize>),
    /// Explicit source rows, scanned in this order.
    Rows(&'a [u32]),
}

impl Selection<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Range(range) => range.len(),
            Self::Rows(rows) => rows.len(),
        }
    }

    /// Positions `start..start + len` of this selection, in scan order.
    pub(crate) fn window(&self, start: usize, len: usize) -> Self {
        match self {
            Self::Range(range) => Self::Range(range.start + start..range.start + start + len),
            Self::Rows(rows) => Self::Rows(&rows[start..start + len]),
        }
    }

    /// Source row at scan position `index`.
    pub(crate) fn row(&self, index: usize) -> usize {
        match self {
            Self::Range(range) => range.start + index,
            Self::Rows(rows) => rows[index] as usize,
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
    }

    impl Fixture {
        pub(crate) fn all(array: &ArrayRef) -> Self {
            Self {
                column: Column::new("test", array.clone()).unwrap(),
                rows: RowIds::All(array.len()),
            }
        }

        pub(crate) fn new(array: &ArrayRef, rows: Vec<u32>) -> Self {
            Self {
                column: Column::new("test", array.clone()).unwrap(),
                rows: RowIds::Selected(Arc::from(rows)),
            }
        }

        pub(crate) fn view(&self) -> ColView<'_> {
            ColView::new(&self.column, self.rows.as_slice())
        }
    }
}
