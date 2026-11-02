//! Ordered column projection and row query configuration.

use super::filter::Filter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColSort {
    pub priority: u32,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColSpec {
    /// Stable index in the dataset schema, independent of display order.
    pub column: u32,
    pub sort: Option<ColSort>,
    pub filter: Option<Filter>,
}

/// Entries are visible columns in display order. Omitted columns have no
/// effect on filtering or sorting. An empty projection displays no columns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuerySpec {
    pub columns: Vec<ColSpec>,
}

impl QuerySpec {
    pub fn all(n_columns: usize) -> Self {
        Self {
            columns: (0..n_columns as u32)
                .map(|column| ColSpec {
                    column,
                    sort: None,
                    filter: None,
                })
                .collect(),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(column: u32, filter: Filter) -> Self {
        Self {
            columns: vec![ColSpec {
                column,
                sort: None,
                filter: Some(filter),
            }],
        }
    }

    pub fn with_filter(mut self, column: u32, filter: Filter) -> Self {
        if let Some(spec) = self.columns.iter_mut().find(|spec| spec.column == column) {
            spec.filter = Some(filter);
        } else {
            self.columns.push(ColSpec {
                column,
                sort: None,
                filter: Some(filter),
            });
        }
        self
    }

    pub fn with_sort(mut self, column: u32, descending: bool, priority: u32) -> Self {
        let sort = Some(ColSort {
            priority,
            descending,
        });
        if let Some(spec) = self.columns.iter_mut().find(|spec| spec.column == column) {
            spec.sort = sort;
        } else {
            self.columns.push(ColSpec {
                column,
                sort,
                filter: None,
            });
        }
        self
    }

    pub(crate) fn sorts(&self) -> Vec<&ColSpec> {
        let mut columns: Vec<_> = self.columns.iter().filter(|c| c.sort.is_some()).collect();
        columns.sort_by_key(|c| c.sort.unwrap().priority);
        columns
    }

    /// Display reordering can reuse the immutable row selection.
    pub(crate) fn same_rows(&self, other: &Self) -> bool {
        fn filters(spec: &QuerySpec) -> std::collections::BTreeMap<u32, &Filter> {
            spec.columns
                .iter()
                .filter_map(|c| c.filter.as_ref().map(|f| (c.column, f)))
                .collect()
        }
        filters(self) == filters(other)
            && self
                .sorts()
                .iter()
                .map(|c| (c.column, c.sort.unwrap().descending))
                .eq(other
                    .sorts()
                    .iter()
                    .map(|c| (c.column, c.sort.unwrap().descending)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewInfo {
    pub version: u64,
    pub row_count: u64,
}

/// A block of formatted cells, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowBlock {
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// `n_rows * n_cols` cells; `None` is a null value.
    pub cells: Vec<Option<String>>,
    /// Row number in the source file for each returned row.
    pub source_rows: Vec<u64>,
}

impl RowBlock {
    pub fn cell(&self, row: u32, col: u32) -> Option<&str> {
        self.cells[(row * self.n_cols + col) as usize].as_deref()
    }
}
