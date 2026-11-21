use super::*;

#[test]
fn null_counts_and_valid_rows_follow_each_selection_kind() {
    let nulls = NullBuffer::from(vec![true, false, true, false, true]);
    let mask = arrow::buffer::BooleanBuffer::from(vec![false, true, true, true, false]);
    let selections = [
        Selection::Range(0..5),
        Selection::Range(1..4),
        Selection::Rows(&[4, 3, 1]),
        Selection::Mask(&mask),
    ];
    for (selection, nulls_seen, valid) in [
        (&selections[0], 2, vec![0, 2, 4]),
        (&selections[1], 2, vec![2]),
        (&selections[2], 2, vec![4]),
        (&selections[3], 2, vec![2]),
    ] {
        assert_eq!(
            null_count(Some(&nulls), selection),
            nulls_seen,
            "{selection:?}"
        );
        let mut rows = Vec::new();
        for_each_valid_row(Some(&nulls), selection, |row| rows.push(row));
        assert_eq!(rows, valid, "{selection:?}");
        let mut all = Vec::new();
        for_each_valid_row(None, selection, |row| all.push(row));
        assert_eq!(all.len(), selection.len(), "{selection:?}");
        assert_eq!(null_count(None, selection), 0);
    }
}
