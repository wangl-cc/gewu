//! Data engine for gewu, a read-only table viewer.
//!
//! The Mac app talks to a [`Session`] through UniFFI. A session manages active views,
//! filters, and display order over an immutable in-memory [`Dataset`]. Sorting and
//! filtering never move data; they only produce a new list of row numbers.

pub mod column;
pub mod dataset;
mod error;
pub mod file;
pub(crate) mod format;
pub(crate) mod query;
pub mod session;
pub mod stats;
pub mod types;

pub use arrow::datatypes::TimeUnit;
pub use column::{
    Bound, Column, ColumnCapabilities, ColumnInfo, DistinctResult, DistinctValue, LogicalType,
};
pub use dataset::Dataset;
pub use error::{Error, Result};
pub use file::{
    xlsx_sheet_names, Export, ExportOptions, ExportProgress, FileInfo, FileMetadata, FileType,
    OpenOptions, ParquetCompression,
};
pub(crate) use query::{engine, predicate, scope, selection};
pub use query::{
    CellMatch, Filter, Find, FindHighlight, FindOperation, QuerySpec, RowBlock, SortKey, ViewInfo,
    MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};
pub use session::{
    FilterEditor, FilterSpec, Session, TextChoice, TextChoiceCursor, TextChoicePage, TextChoices,
    TextFilterMode,
};
pub use stats::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, ArrayView, AxisScale,
    ColumnAnalysis, ColumnSummary, Histogram, LengthSummary, LengthUnit, NumericStatistics,
    RangeBounds, RangeHistogram, ScatterAnalysis, ScatterPoint, ScatterResult, ValueSummary,
    MAX_SCATTER_POINTS,
};
pub use types::Encoding;
