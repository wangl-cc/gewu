//! Column-major display text and source row indices.

use std::fmt::Write;

use super::OrderedView;
use crate::{format, ColumnId, InternalError, Result};

/// Display text packed in column order into one UTF-8 buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedColumnText {
    row_count: u32,
    text: String,
    /// One offset per row plus the final end offset.
    offsets: Vec<u32>,
    /// One bit per row, starting with the low bit; unset is null.
    validity: Vec<u8>,
}

impl FormattedColumnText {
    pub fn row_count(&self) -> u32 {
        self.row_count
    }

    /// Text of a row relative to the segment start; `None` for null or out of range.
    pub fn cell(&self, row: u32) -> Option<&str> {
        self.cell_in_column(0, row)
    }

    pub fn cell_in_column(&self, column: usize, row: u32) -> Option<&str> {
        if row >= self.row_count {
            return None;
        }
        let index = column
            .checked_mul(self.row_count as usize)?
            .checked_add(row as usize)?;
        let end = *self.offsets.get(index.checked_add(1)?)? as usize;
        if self.validity[index / 8] & (1 << (index % 8)) == 0 {
            return None;
        }
        self.text.get(self.offsets[index] as usize..end)
    }

    /// Transfer UTF-8 bytes, row offsets (including the final end offset), and
    /// a validity bitmap with the first row in the low bit.
    pub fn into_parts(self) -> (Vec<u8>, Vec<u32>, Vec<u8>) {
        (self.text.into_bytes(), self.offsets, self.validity)
    }
}

impl OrderedView {
    /// Format one column at display rows `start..start + count`, clipped to
    /// the available rows. Fails for an unknown column, and if the segment's
    /// text exceeds the 4 GiB its 32-bit offsets can address.
    pub fn format_column(
        &self,
        column: ColumnId,
        start: u64,
        count: u32,
    ) -> Result<FormattedColumnText> {
        self.format_columns(&[column], start, count)
    }

    /// Format the requested columns into a single column-major text buffer.
    pub fn format_columns(
        &self,
        columns: &[ColumnId],
        start: u64,
        count: u32,
    ) -> Result<FormattedColumnText> {
        let window = self.row_window(start, count);
        let cells = window.len() * columns.len();
        let mut text = String::new();
        let mut offsets = Vec::with_capacity(cells + 1);
        let mut validity = vec![0; cells.div_ceil(8)];
        offsets.push(0);
        for &column in columns {
            let array = self.column(column)?.array();
            let formatter = format::formatter(array.as_ref())?;
            for row in window.iter() {
                if !array.is_null(row) {
                    // Keep the same display formatting, including inline Arrow errors.
                    write!(text, "{}", formatter.value(row))
                        .expect("formatting into String succeeds");
                    let index = offsets.len() - 1;
                    validity[index / 8] |= 1 << (index % 8);
                }
                let end = u32::try_from(text.len()).map_err(|_| {
                    InternalError::Invariant("formatted column text exceeds 4 GiB".into())
                })?;
                offsets.push(end);
            }
        }
        Ok(FormattedColumnText {
            row_count: window.len() as u32,
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

    #[test]
    fn segments_keep_byte_offsets_nulls_and_display_order() {
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
        let text = ColumnId::from_raw(0);
        let number = ColumnId::from_raw(1);
        let ordered = dataset
            .unfiltered()
            .sort(&[ColumnSort {
                column: number,
                descending: true,
            }])
            .unwrap();
        assert_eq!(ordered.source_rows(1, 64), [8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(
            ordered.format_column(number, 1, 1).unwrap().cell(0),
            Some("8")
        );
        let segment = ordered.format_column(text, 1, 64).unwrap();
        assert_eq!(segment.row_count(), 9, "clipped to the available rows");
        assert_eq!(segment.cell(9), None);
        let actual: Vec<_> = (0..segment.row_count())
            .map(|row| segment.cell(row))
            .collect();
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
