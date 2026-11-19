//! Packed formatted cell text and batched viewport tiles.

use crate::ColumnId;

#[derive(uniffi::Record)]
pub struct CellText {
    pub n_rows: u32,
    pub n_cols: u64,
    /// Shared UTF-8 buffer. Only split if it exceeds the transport's bytes limit.
    pub utf8_chunks: Vec<Vec<u8>>,
    /// One little-endian u64 offset per cell, plus the final end offset.
    /// Cell indices are column-major.
    pub offsets: Vec<u8>,
    /// One validity bit per cell, starting with the low bit.
    pub validity: Vec<u8>,
}

impl From<gewu_core::CellText> for CellText {
    fn from(cells: gewu_core::CellText) -> Self {
        let n_rows = cells.row_count();
        let n_cols = cells.column_count() as u64;
        let (text, offsets, validity) = cells.into_parts();
        // UniFFI prefixes each bytes field with a signed 32-bit length.
        let utf8_chunks = if text.len() <= i32::MAX as usize {
            vec![text]
        } else {
            text.chunks(i32::MAX as usize).map(<[u8]>::to_vec).collect()
        };
        Self {
            n_rows,
            n_cols,
            utf8_chunks,
            offsets: offsets.into_iter().flat_map(u64::to_le_bytes).collect(),
            validity,
        }
    }
}

/// Consecutive display rows whose source indices a [`TileBatch`] returns.
#[derive(uniffi::Record)]
pub struct RowSpan {
    pub start: u64,
    pub count: u32,
}

/// One column over consecutive display rows.
#[derive(uniffi::Record)]
pub struct CellTile {
    pub column: ColumnId,
    pub start: u64,
    pub count: u32,
}

/// Results of one batched viewport read, in request order.
#[derive(uniffi::Record)]
pub struct TileBatch {
    /// Source row indices for each requested [`RowSpan`].
    pub source_rows: Vec<Vec<u64>>,
    /// One single-column [`CellText`] for each requested [`CellTile`].
    pub cells: Vec<CellText>,
}
