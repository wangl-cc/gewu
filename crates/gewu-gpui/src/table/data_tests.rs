use super::*;

fn sample() -> TableData {
    TableData::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workflow.csv")).unwrap()
}

#[test]
fn cached_blocks_are_replaced_with_the_sorted_view() {
    let mut data = sample();
    data.prefetch([0], 0..3);
    assert_eq!(data.cell(0, 1).unwrap().as_deref(), Some("drop"));
    assert!(data.cell(0, 200).is_none());
    assert_eq!(data.cell_preview(0, 1).unwrap().as_deref(), Some("drop"));
    data.prefetch([0], 1..2);
    let block = &data.cache[&(data.columns[0].id, 0)];
    assert!(matches!(&block.text, BlockText::Ready { visible, .. }
        if visible[1].get().is_some()));
    let sort = gewu_core::ColumnSort {
        column: data.columns[1].id,
        descending: true,
    };
    let job = data.edit_query(|q| q.sorts = vec![sort]).unwrap();

    assert!(matches!(
        data.finish_query(job.run()),
        QueryCompletion::Installed
    ));
    assert_eq!(data.query().sorts, [sort]);
    assert!(data.cell(0, 1).is_none());
    data.prefetch([0], 1..2);
    assert_eq!(data.cell(0, 0).unwrap().as_deref(), Some("drop"));
}

#[test]
fn a_failed_visible_block_reports_the_error_without_yielding_cell_text() {
    let mut data = sample();
    let id = data.columns[0].id;
    data.cache.insert(
        (id, 0),
        Block::new(
            Err(gewu_core::Error::from(gewu_core::InternalError::Invariant(
                "fixture".into(),
            ))),
            0,
        ),
    );
    let error = data.prefetch([0], 0..1).unwrap();
    assert!(matches!(
        error,
        gewu_core::Error::Internal(gewu_core::InternalError::Invariant(text)) if text == "fixture"
    ));
    assert!(data.cell_preview(0, 0).is_none());
    assert!(data.cell(0, 0).is_none());
    assert!(data.prefetch([1], 0..1).is_none());
}

#[test]
fn hidden_and_reordered_columns_keep_their_identity_and_manual_width() {
    use crate::table::columns::ColumnWidth;

    let mut table = sample();
    let first = table.columns[0].id;
    let second = table.columns[1].id;
    table.columns.set_width(0, ColumnWidth::Manual(179.));
    table.columns.set_width(1, ColumnWidth::Measured(84.));
    assert!(table.columns.move_to(0, 1));
    let row = gewu_core::ViewRow(0);
    assert_eq!(table.display_cell(row, first).unwrap().column, 1);
    assert!(
        table
            .display_cell(gewu_core::ViewRow(u64::MAX), first)
            .is_none()
    );
    assert!(table.columns.set_visible([first, second].into(), false));
    assert!(table.display_cell(row, first).is_none());
    table.columns.invalidate_measurements();
    assert!(table.columns.set_visible([first, second].into(), true));
    let manual = table.columns.position(first).unwrap().index();
    let automatic = table.columns.position(second).unwrap().index();
    assert_eq!(table.columns.width(manual), 179.);
    assert!(matches!(
        table.columns.entry(automatic).width,
        ColumnWidth::Unmeasured
    ));
}

#[test]
fn copy_uses_core_tsv_export() {
    let data = sample();
    let ids = data.column_ids(0..2);
    let text = copy_tsv(data.view(), 1..3, &ids, true).unwrap();
    assert_eq!(text.lines().collect::<Vec<_>>(), [
        "group\tscore",
        "drop\t99",
        "keep\t5"
    ]);
}

#[test]
fn grid_preview_preserves_utf8_boundaries_and_single_line_while_details_keep_the_original() {
    let long = "猫".repeat(500);
    let short = "short enough for the grid and long enough to own its bytes";
    let input = format!("label\n\"{long}\nsecond line\"\n{short}\n");
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("long.csv");
    std::fs::write(&file, input).unwrap();
    let mut data = TableData::from_dataset(Arc::new(Dataset::open(file).unwrap()));
    data.prefetch([0], 0..2);
    let initial_budget = data.cache_bytes;
    let clipped = data.cell_preview(0, 0).unwrap().unwrap();
    assert!(clipped.ends_with('…') && clipped.len() <= 1027);
    let single_line = data.cell_preview(0, 1).unwrap().unwrap();
    assert_eq!(
        data.cell(0, 1).unwrap().unwrap().as_ptr(),
        single_line.as_ptr()
    );
    assert_eq!(data.cache_bytes, initial_budget);
    let original = data.cell(0, 0).unwrap().unwrap();
    assert_eq!(original.as_ref(), format!("{long}\nsecond line"));
    let full_budget = data.cache_bytes;
    assert!(full_budget >= initial_budget + clipped.len());
    assert_eq!(
        data.cell(0, 0).unwrap().unwrap().as_ptr(),
        original.as_ptr()
    );
    assert_eq!(data.cache_bytes, full_budget);
    assert_eq!(preview("first\r\nsecond").as_ref(), "first…");
}
