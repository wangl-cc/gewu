#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Date32Array, Int32Array, RecordBatch, StringArray};
use gewu_core::{
    CellMatch, ColumnFilter, ColumnHighlight, ColumnId, ColumnSort, Error, Filter, FindCounts,
    OrderedView, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};
use parquet::arrow::ArrowWriter;

fn dataset(columns: Vec<(&str, ArrayRef)>) -> (tempfile::NamedTempFile, Arc<gewu_core::Dataset>) {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let dataset = Arc::new(gewu_core::Dataset::open(file.path()).unwrap());
    (file, dataset)
}

fn document(columns: Vec<(&str, ArrayRef)>) -> (tempfile::NamedTempFile, Arc<OrderedView>) {
    let (file, dataset) = dataset(columns);
    (file, dataset.unfiltered().unsorted())
}

#[test]
fn text_match_ranges_use_the_sessions_literal_unicode_matching() {
    let text = "K.πk.kX K.";
    let (_file, doc) = document(vec![("text", Arc::new(StringArray::from(vec![text])))]);
    let col = ColumnId::from_raw(0);
    let find = doc.find(" k. ", &[col]).unwrap();
    assert!(find.highlight(&[col], 0, 1).unwrap().columns[0].matches(0));
    let ranges = find.text_match_ranges(text, text.len()).collect::<Vec<_>>();
    assert_eq!(ranges, [0..4, 6..8, 11..13]);
    assert_eq!(find.text_match_ranges(text, 3).next(), Some(0..3));
    assert_eq!(find.text_match_ranges(text, 2).next(), None);
}

#[test]
fn arbitrary_find_text_is_not_a_typed_input_error() {
    let (_file, doc) = document(vec![
        ("number", Arc::new(Int32Array::from(vec![1, 2]))),
        ("flag", Arc::new(BooleanArray::from(vec![true, false]))),
        ("date", Arc::new(Date32Array::from(vec![0, 1]))),
    ]);
    let find = doc
        .find("not a typed value", &[
            ColumnId::from_raw(0),
            ColumnId::from_raw(1),
            ColumnId::from_raw(2),
        ])
        .unwrap();
    assert!(find.next(None, 10).unwrap().is_empty());
}

#[test]
fn public_pages_continue_beyond_one_hundred_thousand_in_both_directions() {
    let len = 100_020;
    let values: ArrayRef = Arc::new(StringArray::from_iter_values((0..len).map(|_| "hit")));
    let (_file, doc) = document(vec![("value", values)]);
    let c0 = ColumnId::from_raw(0);
    let find = doc.find("hit", &[c0]).unwrap();

    let cell = CellMatch {
        display_row: 98_300,
        column: c0,
    };
    assert_eq!(
        find.counts_at(cell, || false).unwrap(),
        Some(FindCounts {
            through: 98_301,
            total: len,
        })
    );
    let mut batches = 0;
    assert_eq!(
        find.counts_at(cell, || {
            batches += 1;
            batches == 2
        })
        .unwrap(),
        None
    );

    let mut cursor = None;
    let mut expected = 0;
    loop {
        let page = find.next(cursor.as_ref(), MAX_FIND_PAGE_SIZE).unwrap();
        assert!(page.len() <= MAX_FIND_PAGE_SIZE as usize);
        if page.is_empty() {
            break;
        }
        for hit in page.iter() {
            assert_eq!(hit, &CellMatch {
                display_row: expected,
                column: c0,
            });
            expected += 1;
        }
        cursor = page.last().copied();
    }
    assert_eq!(expected, len);

    let mut cursor = None;
    let mut expected = len - 1;
    let mut seen = 0;
    loop {
        let page = find.previous(cursor.as_ref(), MAX_FIND_PAGE_SIZE).unwrap();
        assert!(page.len() <= MAX_FIND_PAGE_SIZE as usize);
        if page.is_empty() {
            break;
        }
        assert!(page
            .windows(2)
            .all(|pair| pair[0].display_row < pair[1].display_row));
        for hit in page.iter().rev() {
            assert_eq!(hit, &CellMatch {
                display_row: expected,
                column: c0,
            });
            expected = expected.saturating_sub(1);
        }
        seen += page.len() as u64;
        cursor = page.first().copied();
    }
    assert_eq!(seen, len);
    for limit in [0, MAX_FIND_PAGE_SIZE + 1] {
        assert!(matches!(
            find.next(None, limit),
            Err(gewu_core::FindError::InvalidPageSize { .. })
        ));
    }
}

#[test]
fn find_uses_view_order_scope_and_typed_predicates_without_mutating_the_view() {
    let numbers: ArrayRef = Arc::new(Int32Array::from(vec![120, 12, 12, 12]));
    let labels: ArrayRef = Arc::new(StringArray::from(vec!["12", "other", "needle", "needle"]));
    let keep: ArrayRef = Arc::new(BooleanArray::from(vec![true, true, false, true]));
    let (_file, doc) = dataset(vec![("number", numbers), ("label", labels), ("keep", keep)]);
    let (c0, c1, c2) = (
        ColumnId::from_raw(0),
        ColumnId::from_raw(1),
        ColumnId::from_raw(2),
    );
    let view = doc
        .filter(&[ColumnFilter {
            column: c2,
            filter: Filter::exact(vec!["true".into()], false, false),
        }])
        .unwrap()
        .sort(&[ColumnSort {
            column: c0,
            descending: true,
        }])
        .unwrap();
    let find = view.find("12", &[c0, c1]).unwrap();
    assert_eq!(find.next(None, 10).unwrap(), vec![
        CellMatch {
            display_row: 0,
            column: c1,
        },
        CellMatch {
            display_row: 1,
            column: c0,
        },
        CellMatch {
            display_row: 2,
            column: c0,
        },
    ]);
    // One bitmap per requested column, in request order, low bit first.
    let highlight = find.highlight(&[c0, c1], 0, 3).unwrap();
    assert_eq!(highlight.n_rows, 3);
    assert_eq!(highlight.columns, [
        ColumnHighlight {
            column: c0,
            matched: vec![0b110],
        },
        ColumnHighlight {
            column: c1,
            matched: vec![0b001],
        },
    ]);
    // Only the requested (for example on-screen) columns are evaluated.
    assert_eq!(find.highlight(&[c1], 0, 3).unwrap().columns, [highlight
        .columns[1]
        .clone()]);
    assert_eq!(doc.n_rows(), 4);
    assert_eq!(view.source_rows(0, 3), [0, 1, 3]);

    // Captured Find work remains tied to its own version after a newer view.
    let _other = doc.unfiltered();
    assert_eq!(find.highlight(&[c0, c1], 0, 3).unwrap(), highlight);
}

#[test]
fn find_rejects_invalid_scopes_queries_and_highlight_sizes() {
    let values: ArrayRef = Arc::new(StringArray::from_iter_values(["a", "b"]));
    let (_file, doc) = document(vec![("value", values)]);
    let c0 = ColumnId::from_raw(0);
    let c1 = ColumnId::from_raw(1);
    for columns in [vec![c0, c0], vec![c0, c1]] {
        assert!(matches!(
            doc.find("a", &columns),
            Err(Error::Column(_) | Error::Find(_))
        ));
    }
    assert!(matches!(
        doc.find("   ", &[c0]),
        Err(Error::Column(_) | Error::Find(_))
    ));
    let find = doc.find("a", &[c0]).unwrap();
    for count in [0, MAX_FIND_HIGHLIGHT_ROWS + 1] {
        assert!(matches!(
            find.highlight(&[c0], 0, count),
            Err(gewu_core::FindError::InvalidHighlightRows { .. })
        ));
    }
    assert_eq!(
        find.highlight(&[c0], 0, MAX_FIND_HIGHLIGHT_ROWS)
            .unwrap()
            .n_rows,
        2
    );
    assert!(matches!(
        find.highlight(&[c1], 0, 1),
        Err(gewu_core::FindError::InvalidHighlightColumn { column }) if column == c1
    ));
}

#[test]
fn pages_and_cursors_follow_the_captured_column_order() {
    let first: ArrayRef = Arc::new(StringArray::from(vec!["hit", "hit"]));
    let second: ArrayRef = Arc::new(StringArray::from(vec!["hit", "hit"]));
    let (_file, doc) = document(vec![("first", first), ("second", second)]);
    let c0 = ColumnId::from_raw(0);
    let c1 = ColumnId::from_raw(1);
    let find = doc.find("hit", &[c1, c0]).unwrap();
    assert_eq!(
        find.counts_at(
            CellMatch {
                display_row: 1,
                column: c1
            },
            || false
        )
        .unwrap(),
        Some(FindCounts {
            through: 3,
            total: 4
        })
    );
    assert_eq!(find.next(None, 3).unwrap(), [
        CellMatch {
            display_row: 0,
            column: c1,
        },
        CellMatch {
            display_row: 0,
            column: c0,
        },
        CellMatch {
            display_row: 1,
            column: c1,
        },
    ]);
    // Cursor columns are source IDs, not positions in the scope order.
    let after = find.next(
        Some(&CellMatch {
            display_row: 0,
            column: c1,
        }),
        2,
    );
    assert_eq!(after.unwrap(), [
        CellMatch {
            display_row: 0,
            column: c0,
        },
        CellMatch {
            display_row: 1,
            column: c1,
        },
    ]);
    let before = find.previous(
        Some(&CellMatch {
            display_row: 1,
            column: c0,
        }),
        1,
    );
    assert_eq!(before.unwrap(), [CellMatch {
        display_row: 1,
        column: c1,
    }]);
    let highlight = find.highlight(&[c0, c1], 0, 1).unwrap();
    assert!(highlight.columns.iter().all(|column| column.matched == [1]));
}
