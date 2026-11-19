//! Visible row formatting.

use arrow::array::ArrayRef;

use super::OrderedView;
use crate::{format, ColumnId, Result};

/// A block of formatted cells, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowBlock {
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// `n_rows * n_cols` cells; `None` is a null value.
    pub cells: Vec<Option<String>>,
    /// Row number in the source file for each returned row.
    pub source_rows: Vec<u64>,
}

impl RowBlock {
    pub fn cell(&self, row: u32, col: u32) -> Option<&str> {
        self.cells[(row * self.n_cols + col) as usize].as_deref()
    }
}

impl OrderedView {
    /// Formatted cells at display positions, in the requested column order.
    pub fn rows(&self, start: u64, count: u32, columns: &[ColumnId]) -> Result<RowBlock> {
        for &c in columns {
            self.column(c)?;
        }

        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let window = self.rows.as_slice().window(start as usize, end as usize);

        let arrays: Vec<&ArrayRef> = columns
            .iter()
            .map(|&c| self.dataset.columns()[c.index()].array())
            .collect();
        let formatters = columns
            .iter()
            .map(|&c| format::formatter(self.dataset.columns()[c.index()].array().as_ref()))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut cells = Vec::with_capacity(window.len() * columns.len());
        for r in window.iter() {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(a.as_ref(), f, r));
            }
        }

        Ok(RowBlock {
            start,
            n_rows: window.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: window.iter().map(|r| r as u64).collect(),
        })
    }
}
