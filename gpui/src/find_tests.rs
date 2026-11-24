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
            matched: vec![0, 0, 0, 0, 0, 0, 0, 0b1000_0000, 0b0000_0101],
        }],
    });
    let hits: Vec<_> = (0..192)
        .filter(|&row| find.cached_match(column, row) == Some(true))
        .collect();
    assert_eq!(hits, [127, 128, 130]);
}
