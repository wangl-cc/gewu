use super::{ColumnHighlight, ColumnId, FindHighlight, Highlights};

#[test]
fn batched_highlight_uses_relative_rows_across_cache_blocks() {
    let column = ColumnId::from_raw(3);
    let mut find = Highlights::default();
    // A 67-row result starting at display row 64, with matches straddling
    // the cache boundary and a final match in the partial block.
    find.store_highlights(FindHighlight {
        start: 64,
        n_rows: 67,
        columns: vec![ColumnHighlight {
            column,
            matched: vec![0, 0, 0, 0, 0, 0, 0, 0x80, 0x05],
        }],
    });
    let hits: Vec<_> = (0..192).filter(|&row| find.matched(column, row)).collect();
    assert_eq!(hits, [127, 128, 130]);
}
