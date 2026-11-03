//! Ordered column projection and row query configuration.
use gewu_core as core;

use crate::Filter;

#[derive(uniffi::Record)]
pub struct ColSort {
    pub priority: u32,
    pub descending: bool,
}
#[derive(uniffi::Record)]
pub struct ColSpec {
    pub column: u32,
    pub sort: Option<ColSort>,
    pub filter: Option<Filter>,
}
#[derive(uniffi::Record)]
pub struct QuerySpec {
    pub columns: Vec<ColSpec>,
}

#[derive(uniffi::Record)]
pub struct RowBlock {
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major, `n_rows * n_cols` cells; `nil` is null.
    pub cells: Vec<Option<String>>,
    pub source_rows: Vec<u64>,
}

impl From<QuerySpec> for core::QuerySpec {
    fn from(spec: QuerySpec) -> Self {
        Self {
            columns: spec
                .columns
                .into_iter()
                .map(|c| core::ColSpec {
                    column: c.column,
                    filter: c.filter.map(Into::into),
                    sort: c.sort.map(|s| core::ColSort {
                        priority: s.priority,
                        descending: s.descending,
                    }),
                })
                .collect(),
        }
    }
}

impl From<core::RowBlock> for RowBlock {
    fn from(block: core::RowBlock) -> Self {
        Self {
            start: block.start,
            n_rows: block.n_rows,
            n_cols: block.n_cols,
            cells: block.cells,
            source_rows: block.source_rows,
        }
    }
}
