//! Visible row formatting.

use arrow::array::ArrayRef;

use super::View;
use crate::{format, Result, RowBlock};

impl View {
    /// Formatted cells at display positions, in the requested source-column order.
    pub fn rows(&self, start: u64, count: u32, columns: &[u32]) -> Result<RowBlock> {
        for &c in columns {
            self.column(c)?;
        }

        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let source_rows = &self.rows[start as usize..end as usize];

        let arrays: Vec<&ArrayRef> = columns
            .iter()
            .map(|&c| self.dataset.columns[c as usize].array())
            .collect();
        let formatters = columns
            .iter()
            .map(|&c| format::formatter(self.dataset.columns[c as usize].array().as_ref()))
            .collect::<Result<Vec<_>>>()?;

        let mut cells = Vec::with_capacity(source_rows.len() * columns.len());
        for &r in source_rows {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(a.as_ref(), f, r as usize));
            }
        }

        Ok(RowBlock {
            start,
            n_rows: source_rows.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: source_rows.iter().map(|&r| r as u64).collect(),
        })
    }
}
