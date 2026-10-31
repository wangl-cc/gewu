//! Data engine for gewu, a read-only table viewer.
//!
//! The Mac app talks to a [`Document`] through UniFFI. A document owns the
//! decoded data and a current view: the list of source rows to show, in
//! display order. Sorting and filtering never move data; they only produce a
//! new list of row numbers.

mod column;
mod dataset;
mod error;
mod file;
mod filter;
pub(crate) mod query;
pub mod session;
pub mod task;

pub use arrow::datatypes::TimeUnit;
pub use column::{
    bound::Bound,
    histogram::{Histogram, RangeBounds, RangeHistogram},
    ColumnCapabilities, ColumnInfo,
};
pub(crate) use dataset::{format, value_index};
pub use dataset::{AxisScale, Dataset, DistinctResult, DistinctValue, LogicalType};
pub use error::{Error, Result};
pub use file::{
    xlsx_sheet_names, ExportOptions, FileInfo, FileMetadata, FileType, OpenOptions,
    ParquetCompression,
};
pub use filter::{Filter, FilterEditor, FilterSpec, TextFilterMode};
pub(crate) use query::{engine, predicate, scope, selection};
pub use query::{QuerySpec, RowBlock, SortKey, ViewInfo, ViewState};
pub use session::{Document, Session};
pub(crate) use task::analysis;
pub use task::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, CellMatch, ColumnAnalysis,
    ColumnSummary, Export, ExportProgress, Find, FindHighlight, FindOperation, LengthSummary,
    LengthUnit, NumericStatistics, ScatterAnalysis, ScatterPoint, ScatterResult, TextChoice,
    TextChoiceCursor, TextChoicePage, TextChoices, ValueSummary, MAX_FIND_HIGHLIGHT_ROWS,
    MAX_FIND_PAGE_SIZE, MAX_SCATTER_POINTS,
};
