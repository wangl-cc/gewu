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
pub struct ViewInfo {
    pub version: u64,
    pub row_count: u64,
}

/// The committed specification and row version captured together.
#[derive(uniffi::Record)]
pub struct QueryState {
    pub view: ViewInfo,
    pub spec: QuerySpec,
}

/// A view request registered before background execution.
#[derive(uniffi::Object)]
pub struct ViewOperation {
    pub(crate) inner: core::ViewOperation,
}

#[uniffi::export]
impl ViewOperation {
    pub fn run(&self) -> Result<ViewInfo, crate::GewuError> {
        Ok(self.inner.run()?.into())
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }
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

impl From<core::ViewInfo> for ViewInfo {
    fn from(v: core::ViewInfo) -> Self {
        ViewInfo {
            version: v.version,
            row_count: v.row_count,
        }
    }
}

impl From<core::QuerySpec> for QuerySpec {
    fn from(spec: core::QuerySpec) -> Self {
        Self {
            columns: spec
                .columns
                .into_iter()
                .map(|c| ColSpec {
                    column: c.column,
                    filter: c.filter.map(Into::into),
                    sort: c.sort.map(|s| ColSort {
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
            version: block.version,
            start: block.start,
            n_rows: block.n_rows,
            n_cols: block.n_cols,
            cells: block.cells,
            source_rows: block.source_rows,
        }
    }
}
