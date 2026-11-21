//! Column filter and sort specifications across the language boundary.
use gewu_core::{ColumnFilter, ColumnId, ColumnSort, Filter};

#[uniffi::remote(Record)]
pub struct ColumnFilter {
    pub column: ColumnId,
    pub filter: Filter,
}

#[uniffi::remote(Record)]
pub struct ColumnSort {
    pub column: ColumnId,
    pub descending: bool,
}
