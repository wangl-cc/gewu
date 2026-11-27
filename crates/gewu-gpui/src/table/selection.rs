//! Rectangular cell selection: an anchor cell and a moving head cell.

use std::ops::Range;

use gewu_core::ViewRow;

use super::columns::VisibleCol;

/// A displayed cell: a row of the displayed view and a visible column position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub row: ViewRow,
    pub column: VisibleCol,
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

    /// The selected display rows, as the span Core's row ranges take.
    pub fn rows(&self) -> Range<u64> {
        let (a, b) = (self.anchor.row.0, self.head.row.0);
        a.min(b)..a.max(b) + 1
    }

    pub fn contains_row(&self, row: ViewRow) -> bool {
        self.rows().contains(&row.0)
    }

    pub fn columns(&self) -> Range<VisibleCol> {
        let (a, b) = (self.anchor.column, self.head.column);
        a.min(b)..a.max(b).next()
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
        let row = (i128::from(self.head.row.0) + i128::from(rows))
            .clamp(0, row_count.saturating_sub(1) as i128) as u64;
        let head = Cell {
            row: ViewRow(row),
            column: self.head.column.moved(columns, column_count),
        };
        if extend {
            Self {
                anchor: self.anchor,
                head,
            }
        } else {
            Self::single(head)
        }
    }

    pub fn all(row_count: u64, last_column: Option<VisibleCol>) -> Option<Self> {
        let row = row_count.checked_sub(1)?;
        Some(Self {
            anchor: Cell {
                row: ViewRow(0),
                column: VisibleCol::FIRST,
            },
            head: Cell {
                row: ViewRow(row),
                column: last_column?,
            },
        })
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
