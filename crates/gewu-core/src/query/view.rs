//! Query state and formatted view results.

use std::collections::HashMap;

use super::filter::Filter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

/// Declarative specification of what rows to show and how to order them.
///
/// Filters are organized per column (`column_index -> [Filter]`).
/// Across columns and multiple filters on the same column, filters are combined with AND.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuerySpec {
    pub sort: Vec<SortKey>,
    pub filters: HashMap<u32, Vec<Filter>>,
}

impl QuerySpec {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(column: u32, filter: Filter) -> Self {
        Self::default().with_filter(column, filter)
    }

    pub fn with_sort(mut self, sort: Vec<SortKey>) -> Self {
        self.sort = sort;
        self
    }

    pub fn with_filter(mut self, column: u32, filter: Filter) -> Self {
        self.filters.entry(column).or_default().push(filter);
        self
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
