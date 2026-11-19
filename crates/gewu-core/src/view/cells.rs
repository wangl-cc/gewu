//! Formatted cell text, batched viewport tiles and source row indices.

use std::fmt::Write;

use arrow::{array::Array, util::display::ArrayFormatter};

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

/// Consecutive display rows whose source indices a [`TileBatch`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowSpan {
    pub start: u64,
    pub count: u32,
}

/// One column over consecutive display rows, the unit of a batched viewport read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellTile {
    pub column: ColumnId,
    pub start: u64,
    pub count: u32,
}

/// Results of [`OrderedView::read_tiles`], in request order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileBatch {
    /// Source row indices for each requested [`RowSpan`].
    pub source_rows: Vec<Vec<u64>>,
    /// One single-column [`CellText`] for each requested [`CellTile`].
    pub cells: Vec<CellText>,
}

impl OrderedView {
    /// Format cells at display positions in the requested column order.
    pub fn cell_text(&self, start: u64, count: u32, columns: &[ColumnId]) -> Result<CellText> {
        let window = self.row_window(start, count);
        let mut text = CellTextBuilder::new(window.len(), columns.len());
        for &column in columns {
            let array = self.column(column)?.array();
            let formatter = format::formatter(array.as_ref())?;
            text.push_column(array.as_ref(), &formatter, window);
        }
        Ok(text.finish())
    }

    /// Read a viewport's source rows and cell tiles together. Each cell tile is
    /// formatted into its own single-column [`CellText`] and clipped to the
    /// available rows independently, so callers can retain or discard tiles
    /// without sharing a text buffer. Validates every column before formatting.
    pub fn read_tiles(&self, rows: &[RowSpan], cells: &[CellTile]) -> Result<TileBatch> {
        Ok(TileBatch {
            cells: self.cell_tiles(cells)?,
            source_rows: rows
                .iter()
                .map(|span| self.source_rows(span.start, span.count))
                .collect(),
        })
    }

    fn cell_tiles(&self, tiles: &[CellTile]) -> Result<Vec<CellText>> {
        for tile in tiles {
            self.column(tile.column)?;
        }
        let mut result = Vec::with_capacity(tiles.len());
        let mut current: Option<(ColumnId, &dyn Array, ArrayFormatter<'_>)> = None;
        for tile in tiles {
            // Callers usually group tiles by column; reuse the formatter.
            if current
                .as_ref()
                .is_none_or(|(column, ..)| *column != tile.column)
            {
                let array = self.column(tile.column)?.array().as_ref();
                current = Some((tile.column, array, format::formatter(array)?));
            }
            let (_, array, formatter) = current.as_ref().expect("formatter was just created");
            let window = self.row_window(tile.start, tile.count);
            let mut text = CellTextBuilder::new(window.len(), 1);
            text.push_column(*array, formatter, window);
            result.push(text.finish());
        }
        Ok(result)
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

/// Accumulates column-major cells for one [`CellText`].
struct CellTextBuilder {
    n_rows: usize,
    n_cols: usize,
    text: String,
    offsets: Vec<u64>,
    validity: Vec<u8>,
}

impl CellTextBuilder {
    fn new(n_rows: usize, n_cols: usize) -> Self {
        let n_cells = n_rows * n_cols;
        let mut offsets = Vec::with_capacity(n_cells + 1);
        offsets.push(0);
        Self {
            n_rows,
            n_cols,
            text: String::new(),
            offsets,
            validity: vec![0; n_cells.div_ceil(8)],
        }
    }

    fn push_column(
        &mut self,
        array: &dyn Array,
        formatter: &ArrayFormatter<'_>,
        window: crate::row_ids::RowWindow<'_>,
    ) {
        for row in window.iter() {
            if !array.is_null(row) {
                // Display retains Arrow's inline formatting errors.
                write!(self.text, "{}", formatter.value(row))
                    .expect("formatting into String succeeds");
                let index = self.offsets.len() - 1;
                self.validity[index / 8] |= 1 << (index % 8);
            }
            self.offsets.push(self.text.len() as u64);
        }
    }

    fn finish(self) -> CellText {
        debug_assert_eq!(self.offsets.len(), self.n_rows * self.n_cols + 1);
        CellText {
            n_rows: self.n_rows as u32,
            n_cols: self.n_cols,
            text: self.text,
            offsets: self.offsets,
            validity: self.validity,
        }
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

    fn cells(text: &CellText) -> Vec<Option<&str>> {
        (0..text.row_count()).map(|row| text.cell(row, 0)).collect()
    }

    #[test]
    fn tiles_format_independent_clipped_columns_in_request_order() {
        let ordered = sorted_view();
        let text = ColumnId::from_raw(0);
        let number = ColumnId::from_raw(1);
        let tile = |column, start, count| CellTile {
            column,
            start,
            count,
        };
        let batch = ordered
            .read_tiles(
                &[RowSpan { start: 8, count: 4 }, RowSpan {
                    start: 0,
                    count: 2,
                }],
                &[
                    tile(number, 8, 4),
                    tile(text, 0, 4),
                    tile(number, 0, 2),
                    tile(text, 64, 4),
                ],
            )
            .unwrap();
        assert_eq!(batch.source_rows, [vec![1, 0], vec![9, 8]]);
        let tiles = &batch.cells;
        assert!(tiles.iter().all(|t| t.column_count() == 1));
        assert_eq!(cells(&tiles[0]), [Some("1"), Some("0")], "clipped to rows");
        assert_eq!(cells(&tiles[1]), [Some("nine"), None, Some(""), None]);
        assert_eq!(cells(&tiles[2]), [Some("9"), Some("8")]);
        assert_eq!(tiles[3].row_count(), 0);
        // Each tile owns its text; a long value only enlarges its own tile.
        let (bytes, offsets, _) = tiles[2].clone().into_parts();
        assert_eq!((bytes, offsets), (b"98".to_vec(), vec![0, 1, 2]));
        assert!(ordered
            .read_tiles(&[], &[tile(text, 0, 1), tile(ColumnId::from_raw(9), 0, 1)])
            .is_err());
    }
}
