//! Per-column display text and source row indices.

use std::{fmt::Write, ops::Range};

use super::OrderedView;
use crate::{format, ColumnId, InternalError, Result};

/// Display text for one column packed into one UTF-8 buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedColumnSegment {
    text: String,
    /// One offset per row plus the final end offset.
    offsets: Vec<u32>,
    /// One bit per row, starting with the low bit; unset is null.
    validity: Vec<u8>,
}

impl FormattedColumnSegment {
    pub fn row_count(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }

    /// Size of the packed UTF-8 text, excluding offsets and validity bits.
    pub fn text_bytes(&self) -> usize {
        self.text.len()
    }

    /// Text of a row relative to the segment start; `None` for null or out of range.
    pub fn cell(&self, row: u32) -> Option<&str> {
        if row >= self.row_count() {
            return None;
        }
        let row = row as usize;
        if self.validity[row / 8] & (1 << (row % 8)) == 0 {
            return None;
        }
        Some(&self.text[self.offsets[row] as usize..self.offsets[row + 1] as usize])
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        let window = self.row_window(start..start.saturating_add(u64::from(count)));
        let mut text = String::new();
        let mut offsets = Vec::with_capacity(window.len() + 1);
        let mut validity = vec![0; window.len().div_ceil(8)];
        offsets.push(0);
        for (index, row) in window.iter().enumerate() {
            if !crate::column::is_null_at(array.as_ref(), row) {
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
        self.source_rows_in(row..row.saturating_add(1)).next()
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
        self.source_rows_in(start..start.saturating_add(u64::from(count)))
            .map(|source| source.0)
            .collect()
    }

    /// Iterate the source rows of a displayed range in order without copying the indices.
    /// The range is clipped to the available rows.
    pub fn source_rows_in(
        &self,
        displayed: Range<u64>,
    ) -> impl ExactSizeIterator<Item = SourceRow> + '_ {
        self.row_window(displayed)
            .iter()
            .map(|row| SourceRow(row as u64))
    }

    pub(super) fn row_window(&self, displayed: Range<u64>) -> crate::row_ids::RowWindow<'_> {
        self.rows.as_slice().window(
            usize::try_from(displayed.start).unwrap_or(usize::MAX),
            usize::try_from(displayed.end).unwrap_or(usize::MAX),
        )
    }
}

#[cfg(test)]
#[path = "column_segment_tests.rs"]
mod tests;
