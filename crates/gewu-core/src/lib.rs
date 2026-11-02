//! Data engine for gewu, a read-only table viewer.
//!
//! The Mac app talks to a [`Session`] through UniFFI. A session manages active views,
//! filters, and display order over an immutable in-memory [`Dataset`]. Sorting and
//! filtering never move data; they only produce a new list of row numbers.

pub mod column;
pub mod dataset;
mod error;
mod file;
pub(crate) mod format;
pub(crate) mod query;
pub mod session;
pub mod stats;
pub mod task;
pub mod types;

pub use arrow::datatypes::TimeUnit;
pub use column::{Bound, Column, ColumnCapabilities, ColumnInfo, LogicalType};
pub use dataset::Dataset;
pub use error::{Error, Result};
pub use file::{
    xlsx_sheet_names, ExportOptions, FileInfo, FileMetadata, FileType, OpenOptions,
    ParquetCompression,
};
pub(crate) use query::{engine, predicate, scope, selection};
pub use query::{Filter, QuerySpec, RowBlock, SortKey, ViewInfo};
pub use session::{
    DistinctResult, DistinctValue, FilterEditor, FilterSpec, Session, TextFilterMode,
};
pub use stats::{
    ArrayView, AxisScale, ColumnSummary, Histogram, LengthSummary, NumericStatistics, RangeBounds,
    RangeHistogram, ValueSummary,
};
pub use task::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, CellMatch, ColumnAnalysis,
    Export, ExportProgress, Find, FindHighlight, FindOperation, LengthUnit, ScatterAnalysis,
    ScatterPoint, ScatterResult, TextChoice, TextChoiceCursor, TextChoicePage, TextChoices,
    MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE, MAX_SCATTER_POINTS,
};
pub use types::{Encoding, Type};
