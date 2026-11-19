//! Column filter and sort specifications across the language boundary.
use gewu_core as core;

use crate::{ColumnId, Filter};

#[derive(uniffi::Record, Clone)]
pub struct ColumnFilter {
    pub column: ColumnId,
    pub filter: Filter,
}

impl From<ColumnFilter> for core::ColumnFilter {
    fn from(spec: ColumnFilter) -> Self {
        Self {
            column: spec.column.into(),
            filter: spec.filter.into(),
        }
    }
}

#[derive(uniffi::Record, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSort {
    pub column: ColumnId,
    pub descending: bool,
}

impl From<ColumnSort> for core::ColumnSort {
    fn from(spec: ColumnSort) -> Self {
        Self {
            column: spec.column.into(),
            descending: spec.descending,
        }
    }
}
