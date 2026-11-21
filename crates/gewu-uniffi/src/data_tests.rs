use super::*;

#[test]
fn column_text_packs_offsets_validity_and_text() {
    let segment = crate::GewuDataset::open(
        {
            let path =
                std::env::temp_dir().join(format!("gewu-uniffi-pack-{}.csv", std::process::id()));
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
    let path = std::env::temp_dir().join(format!("gewu-uniffi-rows-{}.csv", std::process::id()));
    std::fs::write(&path, "n\n2\n1\n").unwrap();
    let dataset =
        crate::GewuDataset::open(path.to_str().unwrap().to_owned(), Default::default()).unwrap();
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
    let lent = unsafe { gewu_ordered_view_source_rows(Handle::from_arc(sorted.clone()).as_raw()) };
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
