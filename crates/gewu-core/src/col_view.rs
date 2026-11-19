//! A column and the source rows selected by an immutable view.

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

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_complete(&self) -> bool {
        matches!(self.rows, RowSlice::All(_)) || self.rows.len() == self.array().len()
    }

    /// Iterate source rows for order-independent scans. A complete view can
    /// use contiguous addressing even when its display rows are sorted.
    pub(crate) fn iter(&self) -> impl Iterator<Item = usize> + 'a {
        if self.is_complete() {
            RowSlice::All(self.array().len()).iter()
        } else {
            self.rows.iter()
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
