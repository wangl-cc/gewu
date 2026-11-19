//! Formatted cell text and source row indices.

use std::fmt::Write;

use super::OrderedView;
use crate::{format, ColumnId, Result};

/// Formatted cells packed in column order into a shared UTF-8 buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellText {
    n_rows: u32,
    n_cols: usize,
    text: String,
    offsets: Vec<u64>,
    validity: Vec<u8>,
}

impl CellText {
    pub fn row_count(&self) -> u32 {
        self.n_rows
    }

    pub fn column_count(&self) -> usize {
        self.n_cols
    }

    /// Read a cell by its position in the request.
    pub fn cell(&self, row: u32, col: usize) -> Option<&str> {
        if row >= self.n_rows || col >= self.n_cols {
            return None;
        }
        let index = col * self.n_rows as usize + row as usize;
        if self.validity[index / 8] & (1 << (index % 8)) == 0 {
            return None;
        }
        self.text
            .get(self.offsets[index] as usize..self.offsets[index + 1] as usize)
    }

    /// Transfer UTF-8 bytes, cell offsets (including the final end offset),
    /// and a validity bitmap with the first cell in the low bit.
    pub fn into_parts(self) -> (Vec<u8>, Vec<u64>, Vec<u8>) {
        (self.text.into_bytes(), self.offsets, self.validity)
    }
}

impl OrderedView {
    /// Format cells at display positions in the requested column order.
    pub fn cell_text(&self, start: u64, count: u32, columns: &[ColumnId]) -> Result<CellText> {
        let window = self.row_window(start, count);
        let n_cells = window.len() * columns.len();
        let mut text = String::new();
        let mut offsets = Vec::with_capacity(n_cells + 1);
        let mut validity = vec![0; n_cells.div_ceil(8)];
        offsets.push(0);
        for &column in columns {
            let array = self.column(column)?.array();
            let formatter = format::formatter(array.as_ref())?;
            for row in window.iter() {
                if !array.is_null(row) {
                    // Display retains Arrow's inline formatting errors.
                    write!(text, "{}", formatter.value(row))
                        .expect("formatting into String succeeds");
                    let index = offsets.len() - 1;
                    validity[index / 8] |= 1 << (index % 8);
                }
                offsets.push(text.len() as u64);
            }
        }

        Ok(CellText {
            n_rows: window.len() as u32,
            n_cols: columns.len(),
            text,
            offsets,
            validity,
        })
    }

    /// Source row indices in display order, without formatting any cells.
    pub fn source_rows(&self, start: u64, count: u32) -> Vec<u64> {
        self.row_window(start, count)
            .iter()
            .map(|row| row as u64)
            .collect()
    }

    fn row_window(&self, start: u64, count: u32) -> crate::row_ids::RowWindow<'_> {
        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(u64::from(count)).min(len);
        self.rows.as_slice().window(start as usize, end as usize)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Int32Array, RecordBatch, StringArray};

    use super::*;
    use crate::{column::Column, ColumnSort, Dataset, FileMetadata, FileType};

    /// Ten rows sorted by `number` descending: `text` and `number` columns.
    fn sorted_view() -> Arc<OrderedView> {
        let strings = StringArray::from(vec![
            Some("é"),
            None,
            Some(""),
            Some("界🦀"),
            None,
            Some("five"),
            None,
            Some(""),
            None,
            Some("nine"),
        ]);
        let numbers = Int32Array::from_iter_values(0..10);
        let batch = RecordBatch::try_from_iter([
            ("text", Arc::new(strings) as ArrayRef),
            ("number", Arc::new(numbers) as ArrayRef),
        ])
        .unwrap();
        let dataset = Arc::new(Dataset {
            source_path: Default::default(),
            schema: batch.schema(),
            columns: vec![
                Column::new("text", batch.column(0).clone()).unwrap(),
                Column::new("number", batch.column(1).clone()).unwrap(),
            ],
            n_rows: batch.num_rows(),
            file_size: 0,
            format: FileType::Csv,
            metadata: FileMetadata::Delimited,
        });
        dataset
            .unfiltered()
            .sort(&[ColumnSort {
                column: ColumnId::from_raw(1),
                descending: true,
            }])
            .unwrap()
    }

    #[test]
    fn packed_text_keeps_byte_offsets_nulls_and_requested_order() {
        let ordered = sorted_view();
        let text = ColumnId::from_raw(0);
        let number = ColumnId::from_raw(1);
        let block = ordered.cell_text(1, 64, &[number, text]).unwrap();
        assert_eq!(ordered.source_rows(1, 64), [8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(block.cell(0, 0), Some("8"));
        let actual: Vec<_> = (0..block.n_rows).map(|row| block.cell(row, 1)).collect();
        assert_eq!(actual, [
            None,
            Some(""),
            None,
            Some("five"),
            None,
            Some("界🦀"),
            Some(""),
            None,
            Some("é")
        ]);
    }
}
