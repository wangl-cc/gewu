//! Per-column display text and source row indices.

use std::fmt::Write;

use super::OrderedView;
use crate::{format, ColumnId, InternalError, Result};

/// Display text for one column packed into one UTF-8 buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedColumnSegment {
    row_count: u32,
    text: String,
    /// One offset per row plus the final end offset.
    offsets: Vec<u32>,
    /// One bit per row, starting with the low bit; unset is null.
    validity: Vec<u8>,
}

impl FormattedColumnSegment {
    pub fn row_count(&self) -> u32 {
        self.row_count
    }

    /// Size of the packed UTF-8 text, excluding offsets and validity bits.
    pub fn text_bytes(&self) -> usize {
        self.text.len()
    }

    /// Text of a row relative to the segment start; `None` for null or out of range.
    pub fn cell(&self, row: u32) -> Option<&str> {
        if row >= self.row_count {
            return None;
        }
        let row = row as usize;
        if self.validity[row / 8] & (1 << (row % 8)) == 0 {
            return None;
        }
        self.text
            .get(self.offsets[row] as usize..self.offsets[row + 1] as usize)
    }

    /// Transfer UTF-8 bytes, row offsets (including the final end offset), and
    /// a validity bitmap with the first row in the low bit.
    pub fn into_parts(self) -> (Vec<u8>, Vec<u32>, Vec<u8>) {
        (self.text.into_bytes(), self.offsets, self.validity)
    }
}

/// Source row indices of an [`OrderedView`] in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRows<'a> {
    /// Rows `0..n` in file order; display row `i` is source row `i`.
    All(usize),
    /// The source row of each display row.
    Selected(&'a [u32]),
}

/// A zero-based row in the original dataset, independent of filtering or sorting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceRow(pub u64);

/// A zero-based row position in one particular ordered view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRow(pub u64);

impl OrderedView {
    /// Format one column at display rows `start..start + count`, clipped to
    /// the available rows. Fails for an unknown column, and if the segment's
    /// text exceeds the 4 GiB its 32-bit offsets can address.
    pub fn format_column(
        &self,
        column: ColumnId,
        start: u64,
        count: u32,
    ) -> Result<FormattedColumnSegment> {
        let array = self.column(column)?.array();
        let formatter = format::formatter(array.as_ref())?;
        let window = self.row_window(start, count);
        let mut text = String::new();
        let mut offsets = Vec::with_capacity(window.len() + 1);
        let mut validity = vec![0; window.len().div_ceil(8)];
        offsets.push(0);
        for (index, row) in window.iter().enumerate() {
            if !array.is_null(row) {
                // Keep the same display formatting, including inline Arrow errors.
                write!(text, "{}", formatter.value(row)).expect("formatting into String succeeds");
                validity[index / 8] |= 1 << (index % 8);
            }
            let end = u32::try_from(text.len()).map_err(|_| {
                InternalError::Invariant("formatted column text exceeds 4 GiB".into())
            })?;
            offsets.push(end);
        }
        Ok(FormattedColumnSegment {
            row_count: window.len() as u32,
            text,
            offsets,
            validity,
        })
    }

    /// Every source row index in display order, borrowed from the view
    /// without copying. A view of all rows in file order holds no indices.
    pub fn row_ids(&self) -> SourceRows<'_> {
        match &self.rows {
            crate::row_ids::RowIds::All(count) => SourceRows::All(*count),
            crate::row_ids::RowIds::Selected(rows) => SourceRows::Selected(rows),
        }
    }

    /// Resolve a displayed position back to the original dataset.
    pub fn source_row_at(&self, ViewRow(row): ViewRow) -> Option<SourceRow> {
        self.row_window(row, 1)
            .iter()
            .next()
            .map(|row| SourceRow(row as u64))
    }

    /// Find a zero-based source row in this display order without building a reverse index.
    pub fn display_row_for_source(&self, SourceRow(source_row): SourceRow) -> Option<ViewRow> {
        match self.row_ids() {
            SourceRows::All(count) => (source_row < count as u64).then_some(ViewRow(source_row)),
            SourceRows::Selected(rows) => {
                let source = u32::try_from(source_row).ok()?;
                rows.iter()
                    .position(|&row| row == source)
                    .map(|i| ViewRow(i as u64))
            }
        }
    }

    /// Source row indices in display order, without formatting any cells.
    pub fn source_rows(&self, start: u64, count: u32) -> Vec<u64> {
        self.row_window(start, count)
            .iter()
            .map(|row| row as u64)
            .collect()
    }

    pub(super) fn row_window(&self, start: u64, count: u32) -> crate::row_ids::RowWindow<'_> {
        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(u64::from(count)).min(len);
        self.rows.as_slice().window(start as usize, end as usize)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        ArrayRef, Int32Array, ListBuilder, RecordBatch, StringArray, StringBuilder,
    };

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

    #[test]
    fn nested_values_format_as_their_display_text() {
        let mut tags = ListBuilder::new(StringBuilder::new());
        tags.values().append_value("雪");
        tags.values().append_null();
        tags.append(true);
        tags.append(false);
        tags.append(true);
        let tags: ArrayRef = Arc::new(tags.finish());
        let batch = RecordBatch::try_from_iter([("tags", tags)]).unwrap();
        let dataset = Arc::new(Dataset {
            source_path: Default::default(),
            schema: batch.schema(),
            columns: vec![Column::new("tags", batch.column(0).clone()).unwrap()],
            n_rows: batch.num_rows(),
            file_size: 0,
            format: FileType::Parquet,
            metadata: FileMetadata::Delimited,
        });
        let segment = dataset
            .unfiltered()
            .unsorted()
            .format_column(ColumnId::from_raw(0), 0, 64)
            .unwrap();
        let actual: Vec<_> = (0..segment.row_count())
            .map(|row| segment.cell(row))
            .collect();
        assert_eq!(actual, [Some("[雪, ]"), None, Some("[]")]);
    }
}
