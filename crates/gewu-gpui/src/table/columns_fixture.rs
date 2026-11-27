//! Visible columns for pure layout and interaction tests.
use super::{ColumnWidth, TableColumns};

/// `count` columns of `width` points each.
pub(in crate::table) fn columns(count: usize, width: f32) -> TableColumns {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.csv");
    let header = (0..count).map(|c| format!("c{c}")).collect::<Vec<_>>();
    std::fs::write(&path, format!("{}\n", header.join(","))).unwrap();
    let dataset = gewu_core::Dataset::open(&path).unwrap();
    let mut columns = TableColumns::new(dataset.column_infos());
    for index in 0..count {
        columns.set_width(index, ColumnWidth::Manual(width));
    }
    columns
}
