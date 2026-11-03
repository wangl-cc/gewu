//! Data engine for gewu, a read-only table viewer.
//!
//! The Mac app shares immutable [`Dataset`] and [`View`] objects through UniFFI.
//! Callers own query conditions and choose which result to display. Sorting and
//! filtering never move data; they only produce a new list of row numbers.

mod cancellation;
pub use cancellation::Cancellation;
mod column;
mod dataset;
mod error;
mod execution;
mod file;
pub(crate) mod format;
pub(crate) mod query;
mod scalar;
mod stats;
mod types;
mod view;

pub use column::{ColumnCapabilities, ColumnInfo, FilterChoices, ValueCategory, ValueChoice};
pub use dataset::{Dataset, DatasetInfo};
pub use error::{Error, Result};
pub(crate) use execution::{predicate, scope, selection};
pub use file::{
    xlsx_sheet_names, Export, ExportOptions, ExportProgress, FileMetadata, FileType, OpenOptions,
    ParquetCompression,
};
pub use query::{
    CellMatch, ColSort, ColSpec, Filter, FilterCondition, Find, FindHighlight, QuerySpec,
    TextMatch, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};
pub(crate) use scalar::Bound;
pub use stats::{
    AnalysisCapabilities, AnalysisField, AxisScale, Histogram, LengthSummary, LengthUnit,
    NumericStatistics, RangeBound, RangeBounds, RangeHistogram, ScatterAxisKind, ScatterPoint,
    ScatterResult, ValueFrequency, ValueSummary, MAX_SCATTER_POINTS,
};
pub use view::{RowBlock, View};
