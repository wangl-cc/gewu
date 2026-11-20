//! Bulk data for Swift: resident memory lent in place, computed results moved by value.
//!
//! UniFFI can pass values, which it serializes, or opaque object handles, but
//! it cannot lend typed memory. Data that already lives in Rust and may be
//! large is therefore read through the small hand-written C interface below,
//! declared in `include/gewu_data.h`: each function takes a UniFFI object
//! handle and returns a typed pointer into memory that object owns. The
//! memory stays valid, unchanged, for as long as the caller keeps its Swift
//! object alive. Small computed results, such as one block of column text,
//! cross as ordinary UniFFI values instead.

use std::sync::Arc;

use gewu_core as core;
use uniffi::ffi::Handle;

use crate::GewuOrderedView;

/// Display text for one column over consecutive display rows, packed into one
/// buffer: `row_count + 1` native-endian `u32` offsets into the text, then one
/// validity bit per row (low bit first; unset is null), then the UTF-8 text.
#[derive(uniffi::Record)]
pub struct ColumnText {
    pub row_count: u32,
    pub bytes: Vec<u8>,
}

impl From<core::FormattedColumnSegment> for ColumnText {
    fn from(segment: core::FormattedColumnSegment) -> Self {
        let row_count = segment.row_count();
        let (text, offsets, validity) = segment.into_parts();
        let mut bytes = Vec::with_capacity(offsets.len() * 4 + validity.len() + text.len());
        for offset in offsets {
            bytes.extend_from_slice(&offset.to_ne_bytes());
        }
        bytes.extend_from_slice(&validity);
        bytes.extend_from_slice(&text);
        Self { row_count, bytes }
    }
}

/// Bytes produced by an export into memory.
#[derive(uniffi::Object)]
pub struct ExportedBytes {
    bytes: Vec<u8>,
}

impl ExportedBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

/// The source row of each display row of an ordered view. When `rows` is
/// null, display row `i` is source row `i` for every `i < count`.
#[repr(C)]
pub struct GewuSourceRows {
    pub rows: *const u32,
    pub count: usize,
}

/// A read-only byte buffer.
#[repr(C)]
pub struct GewuBytes {
    pub bytes: *const u8,
    pub count: usize,
}

/// Source rows of an ordered view, without copying them.
///
/// # Safety
///
/// `view` must be a handle the caller obtained for a live `GewuOrderedView`
/// by cloning (UniFFI's `uniffiCloneHandle()`); this function consumes it. The
/// returned pointer is valid while the caller's own reference to the view is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gewu_ordered_view_source_rows(view: u64) -> GewuSourceRows {
    // SAFETY: The caller passes a cloned handle of a live `GewuOrderedView`.
    let view = unsafe { Handle::from_raw_unchecked(view).into_arc::<GewuOrderedView>() };
    match view.inner.row_ids() {
        core::SourceRows::All(count) => GewuSourceRows {
            rows: std::ptr::null(),
            count,
        },
        core::SourceRows::Selected(rows) => GewuSourceRows {
            rows: rows.as_ptr(),
            count: rows.len(),
        },
    }
}

/// The contents of an export into memory, without copying them.
///
/// # Safety
///
/// `bytes` must be a handle the caller obtained for a live `ExportedBytes` by
/// cloning (UniFFI's `uniffiCloneHandle()`); this function consumes it. The
/// returned pointer is valid while the caller's own reference to the object is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gewu_exported_bytes(bytes: u64) -> GewuBytes {
    // SAFETY: The caller passes a cloned handle of a live `ExportedBytes`.
    let bytes: Arc<ExportedBytes> = unsafe { Handle::from_raw_unchecked(bytes).into_arc() };
    GewuBytes {
        bytes: bytes.bytes.as_ptr(),
        count: bytes.bytes.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_text_packs_offsets_validity_and_text() {
        let segment = crate::GewuDataset::open(
            {
                let path = std::env::temp_dir()
                    .join(format!("gewu-uniffi-pack-{}.csv", std::process::id()));
                std::fs::write(&path, "t,u\nab,1\n,2\nc,3\n").unwrap();
                path.to_str().unwrap().to_owned()
            },
            Default::default(),
        )
        .unwrap()
        .rows()
        .format_column(core::ColumnId::from_raw(0), 0, 8)
        .unwrap();
        assert_eq!(segment.row_count, 3);
        let offsets: Vec<u32> = segment.bytes[..16]
            .chunks(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect();
        assert_eq!(offsets, [0, 2, 2, 3]);
        assert_eq!(segment.bytes[16], 0b101, "the empty field is null");
        assert_eq!(&segment.bytes[17..], b"abc");
    }

    #[test]
    fn source_rows_lend_the_views_own_indices() {
        let path =
            std::env::temp_dir().join(format!("gewu-uniffi-rows-{}.csv", std::process::id()));
        std::fs::write(&path, "n\n2\n1\n").unwrap();
        let dataset =
            crate::GewuDataset::open(path.to_str().unwrap().to_owned(), Default::default())
                .unwrap();
        let unsorted = dataset.rows();
        let sorted = Arc::clone(&unsorted)
            .apply(vec![], vec![core::ColumnSort {
                column: core::ColumnId::from_raw(0),
                descending: false,
            }])
            .unwrap();
        // SAFETY: Each handle is a fresh strong reference consumed by the call,
        // while `unsorted` and `sorted` keep the views alive.
        let identity =
            unsafe { gewu_ordered_view_source_rows(Handle::from_arc(unsorted.clone()).as_raw()) };
        assert!(identity.rows.is_null() && identity.count == 2);
        let lent =
            unsafe { gewu_ordered_view_source_rows(Handle::from_arc(sorted.clone()).as_raw()) };
        // SAFETY: `sorted` keeps the indices alive.
        assert_eq!(
            unsafe { std::slice::from_raw_parts(lent.rows, lent.count) },
            [1, 0]
        );
        assert_eq!(
            Arc::strong_count(&sorted),
            1,
            "the consumed handle was released"
        );
        std::fs::remove_file(path).unwrap();
    }
}
