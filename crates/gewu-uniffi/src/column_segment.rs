//! Formatted column segments.

use crate::GewuError;

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
