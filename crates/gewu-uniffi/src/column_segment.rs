//! Formatted column segments and batched viewport reads.

use crate::{ColumnId, GewuError};

/// Display text for one column over consecutive display rows.
#[derive(uniffi::Record)]
pub struct FormattedColumnSegment {
    pub row_count: u32,
    /// UTF-8 text of every row, back to back.
    pub utf8: Vec<u8>,
    /// One little-endian u32 offset per row, plus the final end offset.
    pub offsets: Vec<u8>,
    /// One validity bit per row, starting with the low bit; unset is null.
    pub validity: Vec<u8>,
}

impl TryFrom<gewu_core::FormattedColumnSegment> for FormattedColumnSegment {
    type Error = GewuError;

    fn try_from(segment: gewu_core::FormattedColumnSegment) -> Result<Self, GewuError> {
        let row_count = segment.row_count();
        let (utf8, offsets, validity) = segment.into_parts();
        // UniFFI prefixes each bytes field with a signed 32-bit length.
        if utf8.len() > i32::MAX as usize {
            return Err(GewuError::Internal {
                message: "a formatted column segment exceeds 2 GiB".into(),
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

/// One column over consecutive display rows.
#[derive(uniffi::Record)]
pub struct ColumnSpan {
    pub column: ColumnId,
    pub start: u64,
    pub count: u32,
}

/// Results of one batched viewport read, in request order.
#[derive(uniffi::Record)]
pub struct SegmentBatch {
    /// Source row indices for each requested [`RowSpan`].
    pub source_rows: Vec<Vec<u64>>,
    /// One segment for each requested [`ColumnSpan`].
    pub segments: Vec<FormattedColumnSegment>,
}
