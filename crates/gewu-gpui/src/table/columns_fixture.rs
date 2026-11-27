//! Visible columns and cells for pure layout and interaction tests.
use gewu_core::ViewRow;

use super::{ColumnWidth, TableColumns, VisibleCol};
use crate::table::selection::Cell;

/// `count` columns of `width` points each.
pub(in crate::table) fn columns(count: usize, width: f32) -> TableColumns {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.csv");
    let header = (0..count).map(|c| format!("c{c}")).collect::<Vec<_>>();
    std::fs::write(&path, format!("{}\n", header.join(","))).unwrap();
    let dataset = gewu_core::Dataset::open(&path).unwrap();
    let mut columns = TableColumns::new(dataset.column_infos());
    for column in columns.positions() {
        columns.set_width(column, ColumnWidth::Manual(width));
    }
    columns
}

/// The visible column at `index`, as a test names it.
pub(in crate::table) fn col(index: usize) -> VisibleCol {
    VisibleCol(index)
}

/// The cell at display `row` in the visible column at `column`.
pub(in crate::table) fn cell(row: u64, column: usize) -> Cell {
    Cell {
        row: ViewRow(row),
        column: col(column),
    }
}
