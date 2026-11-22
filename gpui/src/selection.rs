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
        let column = (self.head.column as i64 + columns)
            .clamp(0, column_count.saturating_sub(1) as i64) as usize;
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

    pub fn all(row_count: u64, column_count: usize) -> Self {
        Self {
            anchor: Cell { row: 0, column: 0 },
            head: Cell {
                row: row_count.saturating_sub(1),
                column: column_count.saturating_sub(1),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_and_collapse() {
        let s = Selection::single(Cell { row: 5, column: 2 });
        let s = s.moved(3, -1, true, 100, 10);
        assert_eq!(s.rows(), 5..9);
        assert_eq!(s.columns(), 1..3);
        let s = s.moved(-100, 100, false, 100, 10);
        assert_eq!(s, Selection::single(Cell { row: 0, column: 9 }));
        let s = s.moved(i64::MAX, 0, true, 50_000_000, 10);
        assert_eq!(s.head.row, 49_999_999);
    }
}
