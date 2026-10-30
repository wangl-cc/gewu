//! Query state and formatted view results.

use crate::Filter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

/// What the user wants to see. Filters are combined with AND.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    pub sort: Vec<SortKey>,
    pub filters: Vec<Filter>,
    /// Keep rows where any column matches. What "matches" means depends on
    /// the column: substring for text, value or period for numbers, dates
    /// and times (see the `column` module).
    pub search: Option<String>,
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
    /// With a search active, `n_rows * n_cols` flags (row-major) for the
    /// cells that match it; empty otherwise.
    pub matched: Vec<bool>,
}

impl RowBlock {
    pub fn cell(&self, row: u32, col: u32) -> Option<&str> {
        self.cells[(row * self.n_cols + col) as usize].as_deref()
    }

    /// Whether this cell matches the view's search.
    pub fn is_match(&self, row: u32, col: u32) -> bool {
        self.matched
            .get((row * self.n_cols + col) as usize)
            .copied()
            .unwrap_or(false)
    }
}
