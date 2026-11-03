#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Int32Array, RecordBatch, StringArray};
use gewu_core::{
    Cancellation, CellMatch, Error, Filter, QuerySpec, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
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

fn document(columns: Vec<(&str, ArrayRef)>) -> (tempfile::NamedTempFile, Arc<gewu_core::View>) {
    let (file, dataset) = dataset(columns);
    (file, dataset.view())
}

#[test]
fn public_pages_continue_beyond_one_hundred_thousand_in_both_directions() {
    let len = 100_020;
    let values: ArrayRef = Arc::new(StringArray::from_iter_values((0..len).map(|_| "hit")));
    let (_file, doc) = document(vec![("value", values)]);
    let find = doc.find("hit", &[0]).unwrap();

    let mut cursor = None;
    let mut expected = 0;
    loop {
        let page = find
            .next(
                cursor.as_ref(),
                MAX_FIND_PAGE_SIZE,
                &Cancellation::default(),
            )
            .unwrap();
        assert!(page.len() <= MAX_FIND_PAGE_SIZE as usize);
        if page.is_empty() {
            break;
        }
        for hit in page.iter() {
            assert_eq!(hit, &CellMatch {
                display_row: expected,
                column: 0,
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
        let page = find
            .previous(
                cursor.as_ref(),
                MAX_FIND_PAGE_SIZE,
                &Cancellation::default(),
            )
            .unwrap();
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
                column: 0,
            });
            expected = expected.saturating_sub(1);
        }
        seen += page.len() as u64;
        cursor = page.first().copied();
    }
    assert_eq!(seen, len);
    for limit in [0, MAX_FIND_PAGE_SIZE + 1] {
        assert!(matches!(
            find.next(None, limit, &Cancellation::default()),
            Err(Error::InvalidArgument(_))
        ));
    }
}

#[test]
fn find_uses_view_order_scope_and_typed_predicates_without_mutating_the_view() {
    let numbers: ArrayRef = Arc::new(Int32Array::from(vec![120, 12, 12, 12]));
    let labels: ArrayRef = Arc::new(StringArray::from(vec!["12", "other", "needle", "needle"]));
    let keep: ArrayRef = Arc::new(BooleanArray::from(vec![true, true, false, true]));
    let (_file, doc) = dataset(vec![("number", numbers), ("label", labels), ("keep", keep)]);
    let view = doc
        .query(
            &QuerySpec::single(2, Filter::exact(vec!["true".into()], false, false))
                .with_sort(0, true, 0),
            &gewu_core::Cancellation::default(),
        )
        .unwrap();
    let find = view.find("12", &[0, 1]).unwrap();
    assert_eq!(
        find.next(None, 10, &Cancellation::default()).unwrap(),
        vec![
            CellMatch {
                display_row: 0,
                column: 1,
            },
            CellMatch {
                display_row: 1,
                column: 0,
            },
            CellMatch {
                display_row: 2,
                column: 0,
            },
        ]
    );
    let highlight = find.highlight(0, 3, &Cancellation::default()).unwrap();
    assert_eq!(highlight.n_cols, 2);
    assert_eq!(highlight.matched, [false, true, true, false, true, false]);
    assert_eq!(doc.n_rows(), 4);
    assert_eq!(view.rows(0, 3, &[0]).unwrap().source_rows, [0, 1, 3]);

    // Captured Find work remains tied to its own version after a newer view.
    doc.query(&QuerySpec::default(), &gewu_core::Cancellation::default())
        .unwrap();
    assert_eq!(
        find.highlight(0, 3, &Cancellation::default())
            .unwrap()
            .matched,
        highlight.matched
    );
}

#[test]
fn find_rejects_invalid_scopes_queries_and_highlight_sizes() {
    let values: ArrayRef = Arc::new(StringArray::from_iter_values(["a", "b"]));
    let (_file, doc) = document(vec![("value", values)]);
    for columns in [vec![0, 0], vec![0, 1]] {
        assert!(matches!(
            doc.find("a", &columns),
            Err(Error::InvalidArgument(_))
        ));
    }
    assert!(matches!(
        doc.find("   ", &[0]),
        Err(Error::InvalidArgument(_))
    ));
    let find = doc.find("a", &[0]).unwrap();
    for count in [0, MAX_FIND_HIGHLIGHT_ROWS + 1] {
        assert!(matches!(
            find.highlight(0, count, &Cancellation::default()),
            Err(Error::InvalidArgument(_))
        ));
    }
    assert_eq!(
        find.highlight(0, MAX_FIND_HIGHLIGHT_ROWS, &Cancellation::default())
            .unwrap()
            .n_rows,
        2
    );
}

#[test]
fn pages_and_cursors_follow_the_captured_column_order() {
    let first: ArrayRef = Arc::new(StringArray::from(vec!["hit", "hit"]));
    let second: ArrayRef = Arc::new(StringArray::from(vec!["hit", "hit"]));
    let (_file, doc) = document(vec![("first", first), ("second", second)]);
    let find = doc.find("hit", &[1, 0]).unwrap();
    assert_eq!(find.next(None, 3, &Cancellation::default()).unwrap(), [
        CellMatch {
            display_row: 0,
            column: 1,
        },
        CellMatch {
            display_row: 0,
            column: 0,
        },
        CellMatch {
            display_row: 1,
            column: 1,
        },
    ]);
    // Cursor columns are source indexes, not positions in the scope order.
    let after = find.next(
        Some(&CellMatch {
            display_row: 0,
            column: 1,
        }),
        2,
        &Cancellation::default(),
    );
    assert_eq!(after.unwrap(), [
        CellMatch {
            display_row: 0,
            column: 0,
        },
        CellMatch {
            display_row: 1,
            column: 1,
        },
    ]);
    let before = find.previous(
        Some(&CellMatch {
            display_row: 1,
            column: 0,
        }),
        1,
        &Cancellation::default(),
    );
    assert_eq!(before.unwrap(), [CellMatch {
        display_row: 1,
        column: 1,
    }]);
    assert_eq!(
        find.highlight(0, 1, &Cancellation::default())
            .unwrap()
            .matched,
        [true, true]
    );
}

#[test]
fn cancelling_a_stale_snapshot_does_not_cancel_the_current_find() {
    let values: ArrayRef = Arc::new(StringArray::from_iter_values(["old", "new", "new"]));
    let (_file, doc) = document(vec![("value", values)]);
    let stale = doc.find("old", &[0]).unwrap();
    let current = doc.find("new", &[0]).unwrap();
    stale.cancel();
    assert!(matches!(
        stale.next(None, 1, &Cancellation::default()),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        stale.highlight(0, 1, &Cancellation::default()),
        Err(Error::Cancelled)
    ));
    assert_eq!(current.next(None, 1, &Cancellation::default()).unwrap(), [
        CellMatch {
            display_row: 1,
            column: 0,
        }
    ]);
    assert_eq!(
        current.previous(None, 1, &Cancellation::default()).unwrap(),
        [CellMatch {
            display_row: 2,
            column: 0,
        }]
    );
}

#[test]
fn obsolete_operation_cancellation_leaves_new_navigation_and_highlights_usable() {
    let values: ArrayRef = Arc::new(StringArray::from_iter_values(["hit", "hit"]));
    let (_file, doc) = document(vec![("value", values)]);
    let find = doc.find("hit", &[0]).unwrap();
    let obsolete = Cancellation::default();
    obsolete.cancel();
    assert!(matches!(
        find.next(None, 1, &obsolete),
        Err(Error::Cancelled)
    ));
    let navigation = Cancellation::default();
    assert_eq!(find.next(None, 1, &navigation).unwrap(), [CellMatch {
        display_row: 0,
        column: 0,
    }]);
    let highlight = Cancellation::default();
    assert_eq!(find.highlight(0, 2, &highlight).unwrap().matched, [
        true, true
    ]);
}
