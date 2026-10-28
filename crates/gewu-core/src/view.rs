//! Plain data types shared by every front end.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub row_groups: u32,
    pub file_size: u64,
    /// Compression codecs used in the file, e.g. `["SNAPPY"]`.
    pub compression: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// Keep rows whose value is one of `keys` (keys come from
    /// [`crate::Document::distinct_values`]). This is the Excel-style
    /// checkbox filter.
    InValues { column: u32, keys: Vec<u64> },
    /// Keep rows where the value is null (or not null when `negate`).
    IsNull { column: u32, negate: bool },
    /// Keep rows whose displayed text contains `text`.
    Contains {
        column: u32,
        text: String,
        case_sensitive: bool,
    },
}

impl Filter {
    pub fn column(&self) -> u32 {
        match self {
            Filter::InValues { column, .. }
            | Filter::IsNull { column, .. }
            | Filter::Contains { column, .. } => *column,
        }
    }
}

/// What the user wants to see. Filters are combined with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewState {
    pub sort: Vec<SortKey>,
    pub filters: Vec<Filter>,
    /// Case-insensitive search across all columns.
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
}

impl RowBlock {
    pub fn cell(&self, row: u32, col: u32) -> Option<&str> {
        self.cells[(row * self.n_cols + col) as usize].as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Stable for the lifetime of the document; pass it to [`Filter::InValues`].
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnStats {
    pub null_count: u64,
    /// Distinct non-null values.
    pub distinct_count: u64,
    pub min: Option<String>,
    pub max: Option<String>,
}
