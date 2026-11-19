//! Formatted column segments and batched viewport reads.

use crate::{ColumnId, GewuError};

/// Display text packed by column over consecutive display rows.
#[derive(uniffi::Record)]
pub struct FormattedColumnText {
    pub row_count: u32,
    /// UTF-8 text of every row, back to back.
    pub utf8: Vec<u8>,
    /// One little-endian u32 offset per cell, plus the final end offset.
    pub offsets: Vec<u8>,
    /// One validity bit per cell, starting with the low bit; unset is null.
    pub validity: Vec<u8>,
}

impl TryFrom<gewu_core::FormattedColumnText> for FormattedColumnText {
    type Error = GewuError;

    fn try_from(segment: gewu_core::FormattedColumnText) -> Result<Self, GewuError> {
        let row_count = segment.row_count();
        let (utf8, offsets, validity) = segment.into_parts();
        // UniFFI prefixes each bytes field with a signed 32-bit length.
        if utf8.len() > i32::MAX as usize {
            return Err(GewuError::Internal {
                message: "formatted column text exceeds 2 GiB".into(),
            });
        }
        Ok(Self {
            row_count,
            utf8,
            offsets: offsets.into_iter().flat_map(u32::to_le_bytes).collect(),
            validity,
        })
    }
}

/// Consecutive display rows.
#[derive(uniffi::Record)]
pub struct RowSpan {
    pub start: u64,
    pub count: u32,
}

/// Columns to format together over consecutive display rows.
#[derive(uniffi::Record)]
pub struct ColumnBatch {
    pub columns: Vec<ColumnId>,
    pub start: u64,
    pub count: u32,
}

/// Results of one batched viewport read, in request order.
#[derive(uniffi::Record)]
pub struct SegmentBatch {
    /// Source row indices for each requested [`RowSpan`].
    pub source_rows: Vec<Vec<u64>>,
    /// One shared buffer for each requested [`ColumnBatch`].
    pub batches: Vec<FormattedColumnText>,
}
