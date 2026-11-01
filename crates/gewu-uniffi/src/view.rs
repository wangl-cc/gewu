//! Query state and visible row blocks.

use std::collections::HashMap;

use gewu_core as core;

use crate::Filter;

#[derive(uniffi::Record)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

#[derive(uniffi::Record)]
pub struct QuerySpec {
    pub sort: Vec<SortKey>,
    pub filters: HashMap<u32, Vec<Filter>>,
}

#[derive(uniffi::Record)]
pub struct ViewInfo {
    pub version: u64,
    pub row_count: u64,
}

#[derive(uniffi::Record)]
pub struct RowBlock {
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major, `n_rows * n_cols` cells; `nil` is null.
    pub cells: Vec<Option<String>>,
    pub source_rows: Vec<u64>,
}

impl From<QuerySpec> for core::QuerySpec {
    fn from(s: QuerySpec) -> Self {
        core::QuerySpec {
            sort: s
                .sort
                .into_iter()
                .map(|k| core::SortKey {
                    column: k.column,
                    descending: k.descending,
                })
                .collect(),
            filters: s
                .filters
                .into_iter()
                .map(|(col, filters)| (col, filters.into_iter().map(Into::into).collect()))
                .collect(),
        }
    }
}

impl From<core::ViewInfo> for ViewInfo {
    fn from(v: core::ViewInfo) -> Self {
        ViewInfo {
            version: v.version,
            row_count: v.row_count,
        }
    }
}

impl From<core::RowBlock> for RowBlock {
    fn from(block: core::RowBlock) -> Self {
        Self {
            version: block.version,
            start: block.start,
            n_rows: block.n_rows,
            n_cols: block.n_cols,
            cells: block.cells,
            source_rows: block.source_rows,
        }
    }
}
