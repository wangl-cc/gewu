//! Filter and sort input specifications.

pub mod filter;

pub use filter::{Filter, FilterCondition, TextMatch};

use crate::ColumnId;

/// Filter applied to a specific column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnFilter {
    pub column: ColumnId,
    pub filter: Filter,
}

/// Sort key applied to a specific column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSort {
    pub column: ColumnId,
    pub descending: bool,
}
