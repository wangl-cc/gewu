//! Rectangular cell selection: an anchor cell and a moving head cell.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub row: u64,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cell,
    pub head: Cell,
}

impl Selection {
    pub fn single(cell: Cell) -> Self {
        Self {
            anchor: cell,
            head: cell,
        }
    }

    pub fn rows(&self) -> Range<u64> {
        self.anchor.row.min(self.head.row)..self.anchor.row.max(self.head.row) + 1
    }

    pub fn columns(&self) -> Range<usize> {
        self.anchor.column.min(self.head.column)..self.anchor.column.max(self.head.column) + 1
    }

    /// Move the head by `rows` x `columns`, clamped to the table; without
    /// `extend` the selection collapses to the new head.
    pub fn moved(
        self,
        rows: i64,
        columns: i64,
        extend: bool,
        row_count: u64,
        column_count: usize,
    ) -> Self {
        let row = (self.head.row as i128 + rows as i128)
            .clamp(0, row_count.saturating_sub(1) as i128) as u64;
        let column = (self.head.column as i128 + i128::from(columns))
            .clamp(0, column_count.saturating_sub(1) as i128) as usize;
        let head = Cell { row, column };
        if extend {
            Self {
                anchor: self.anchor,
                head,
            }
        } else {
            Self::single(head)
        }
    }

    pub fn all(row_count: u64, column_count: usize) -> Option<Self> {
        let row = row_count.checked_sub(1)?;
        let column = column_count.checked_sub(1)?;
        Some(Self {
            anchor: Cell { row: 0, column: 0 },
            head: Cell { row, column },
        })
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
