//! Bulk results shared with Swift without serialization.
//!
//! UniFFI copies `Vec<u8>` return values byte by byte into a Swift `Data`,
//! and decodes integer vectors element by element. The objects here instead
//! keep their buffers in Rust and expose each buffer's address and length.
//! Swift reads the memory directly for as long as it holds the object; the
//! buffers are never mutated or reallocated after construction.

use gewu_core as core;

use crate::GewuError;

/// The location of an immutable buffer owned by the object that returned it.
/// Valid only while the Swift wrapper of that object is alive.
#[derive(uniffi::Record, Clone, Copy)]
pub struct BufferView {
    pub address: u64,
    /// Length in bytes.
    pub length: u64,
}

impl BufferView {
    fn of<T>(items: &[T]) -> Self {
        Self {
            address: items.as_ptr() as usize as u64,
            length: std::mem::size_of_val(items) as u64,
        }
    }
}

/// Display text for one column over consecutive display rows.
#[derive(uniffi::Object)]
pub struct ColumnText {
    row_count: u32,
    text: Vec<u8>,
    offsets: Vec<u32>,
    validity: Vec<u8>,
}

#[uniffi::export]
impl ColumnText {
    pub fn row_count(&self) -> u32 {
        self.row_count
    }

    /// UTF-8 text of every row, back to back.
    pub fn text(&self) -> BufferView {
        BufferView::of(&self.text)
    }

    /// One native-endian `u32` byte offset into `text` per row, plus the final end offset.
    pub fn offsets(&self) -> BufferView {
        BufferView::of(&self.offsets)
    }

    /// One validity bit per row, starting with the low bit; unset is null.
    pub fn validity(&self) -> BufferView {
        BufferView::of(&self.validity)
    }
}

impl From<core::FormattedColumnSegment> for ColumnText {
    fn from(segment: core::FormattedColumnSegment) -> Self {
        let row_count = segment.row_count();
        let (text, offsets, validity) = segment.into_parts();
        Self {
            row_count,
            text,
            offsets,
            validity,
        }
    }
}

/// Zero-based source row indices in display order.
#[derive(uniffi::Object)]
pub struct RowIndices {
    rows: Vec<u32>,
}

#[uniffi::export]
impl RowIndices {
    pub fn count(&self) -> u32 {
        self.rows.len() as u32
    }

    /// One native-endian `u32` per row.
    pub fn rows(&self) -> BufferView {
        BufferView::of(&self.rows)
    }
}

impl TryFrom<Vec<u64>> for RowIndices {
    type Error = GewuError;

    /// Datasets are limited to `u32::MAX` rows, so every source row fits.
    fn try_from(rows: Vec<u64>) -> Result<Self, GewuError> {
        let rows = rows
            .into_iter()
            .map(u32::try_from)
            .collect::<Result<_, _>>()
            .map_err(|_| GewuError::Internal {
                message: "source row index exceeds u32".into(),
            })?;
        Ok(Self { rows })
    }
}

#[cfg(test)]
impl ColumnText {
    pub(crate) fn utf8(&self) -> &[u8] {
        &self.text
    }

    pub(crate) fn offset_values(&self) -> &[u32] {
        &self.offsets
    }
}

#[cfg(test)]
impl RowIndices {
    pub(crate) fn values(&self) -> &[u32] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_describe_the_owned_buffers() {
        let text = ColumnText {
            row_count: 2,
            text: b"ab".to_vec(),
            offsets: vec![0, 1, 2],
            validity: vec![0b11],
        };
        let offsets = text.offsets();
        assert_eq!(offsets.address, text.offsets.as_ptr() as usize as u64);
        assert_eq!(offsets.length, 12);
        assert_eq!(text.text().length, 2);
        assert_eq!(text.validity().length, 1);
        let rows = RowIndices::try_from(vec![3, 1]).unwrap();
        assert_eq!((rows.count(), rows.rows().length), (2, 8));
    }

    #[test]
    fn empty_buffers_have_zero_length() {
        let rows = RowIndices::try_from(Vec::new()).unwrap();
        assert_eq!(rows.rows().length, 0);
    }
}
