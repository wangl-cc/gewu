//! Plain data types shared by every front end.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    /// The Arrow type, e.g. `Timestamp(Microsecond, Some("UTC"))`.
    pub type_name: String,
    /// How gewu displays the values. Query supported operations through
    /// `Document::column_capabilities`.
    pub kind: crate::DataKind,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub file_size: u64,
    pub format: crate::FileType,
    pub metadata: crate::FileMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Equal to text parsed in this column's own type. Strings compare
    /// literally (case-sensitive); floats parse to their native width;
    /// decimal coefficients remain exact. Invalid input is an error.
    Equals { column: u32, value: String },
    /// Inclusive range with endpoints already parsed for the column's type.
    /// Unlike search, the number of typed decimal places does not widen
    /// these bounds. Missing ends are open; invalid or reversed bounds error.
    Range {
        column: u32,
        min: Option<crate::Bound>,
        max: Option<crate::Bound>,
    },
    /// Keep rows whose value is one of `keys` (keys come from
    /// [`crate::Document::distinct_values`]). This is the Excel-style
    /// checkbox filter.
    InValues { column: u32, keys: Vec<u64> },
    /// Keep rows where the value is null (or not null when `negate`).
    IsNull { column: u32, negate: bool },
    /// Keep rows whose displayed text contains `text`. For text, lists and
    /// other non-ordered columns; numbers, dates, times and booleans are
    /// refused with [`crate::Error::InvalidArgument`] (use `Range` or
    /// `InValues`).
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
            | Filter::Contains { column, .. }
            | Filter::Equals { column, .. }
            | Filter::Range { column, .. } => *column,
        }
    }
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
    /// Whether the keys name values for [`Filter::InValues`]. False for
    /// columns with too many distinct values to index; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

/// Distribution of a numeric column, for the range filter.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    /// Axis of equally spaced bins over the whole source column, not just the
    /// counted rows, so it stays put while filters change. Infinities count
    /// in the end bins. `None` when the column has no finite values.
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// Integer column (including dates and times). Then `hi` is max + 1,
    /// and when the span is small each bin is exactly one integer.
    pub integer: bool,
    pub counts: Vec<u64>,
    pub null_count: u64,
    pub nan_count: u64,
    /// Rows counted (including nulls and NaN).
    pub total: u64,
}

/// Facts about one column over the rows of the current view (so filters
/// and search apply), for the inspector.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSummary {
    /// Rows in the current view.
    pub rows: u64,
    pub null_count: u64,
    pub nan_count: u64,
    /// Distinct non-null values.
    pub distinct_count: u64,
    /// Numbers, dates and times only (dates and times as their physical
    /// value; see `Document::format_value`). These f64 results can approximate
    /// large integers and decimals; distinct counts use the native values.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Smallest and largest non-null values of any comparable column, as
    /// the table displays them.
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    /// Other columns: the most common values, most common first.
    pub top_values: Vec<DistinctValue>,
}

/// The editor and data a frontend needs for one column's filter.
/// The editor is chosen from the whole source column and stays stable as
/// other filters change. Counts exclude this column's own filters.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterSpec {
    pub nullable: bool,
    pub editor: FilterEditor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterEditor {
    Range {
        kind: crate::DataKind,
        distribution: Option<RangeHistogram>,
    },
    /// All source values, including values with zero rows in the current facet.
    Choices {
        values: Vec<DistinctValue>,
    },
    Contains,
    NullOnly,
}

/// Drawing coordinates and exact filter endpoints for the same bins.
/// Coordinates can be approximate; submit `ranges` to filter by a bin.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeHistogram {
    /// Exact source extrema for display, independent of approximate axis coordinates.
    pub min_label: String,
    pub max_label: String,
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeBounds {
    pub min: Option<crate::Bound>,
    pub max: Option<crate::Bound>,
}
