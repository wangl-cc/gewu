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
