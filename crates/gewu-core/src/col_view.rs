//! A column and the source rows selected by an immutable View.

use arrow::{array::ArrayRef, datatypes::DataType};

use crate::column::Column;

#[derive(Clone, Copy)]
pub(crate) struct ColView<'a> {
    column: &'a Column,
    rows: &'a [u32],
}

impl<'a> ColView<'a> {
    pub(crate) fn new(column: &'a Column, rows: &'a [u32]) -> Self {
        Self { column, rows }
    }

    pub(crate) fn column(&self) -> &Column {
        self.column
    }

    pub(crate) fn array(&self) -> &ArrayRef {
        self.column.array()
    }

    pub(crate) fn rows(&self) -> &[u32] {
        self.rows
    }

    pub(crate) fn data_type(&self) -> &DataType {
        self.array().data_type()
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.rows.len() == self.array().len()
    }

    /// Iterate source rows for order-independent scans. A complete View can
    /// use contiguous addressing even when its display rows are sorted.
    pub(crate) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let (all, rows) = if self.is_complete() {
            (0..self.array().len(), &[][..])
        } else {
            (0..0, self.rows)
        };
        all.chain(rows.iter().map(|&row| row as usize))
    }
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    pub(crate) struct Fixture {
        column: Column,
        rows: Vec<u32>,
    }

    impl Fixture {
        pub(crate) fn all(array: &ArrayRef) -> Self {
            Self::new(array, (0..array.len() as u32).collect())
        }

        pub(crate) fn new(array: &ArrayRef, rows: Vec<u32>) -> Self {
            Self {
                column: Column::new("test", array.clone()).unwrap(),
                rows,
            }
        }

        pub(crate) fn view(&self) -> ColView<'_> {
            ColView::new(&self.column, &self.rows)
        }
    }
}
