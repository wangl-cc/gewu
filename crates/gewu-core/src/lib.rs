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
mod file;
pub(crate) mod format;
pub(crate) mod query;
mod stats;
mod types;
mod view;

pub use column::{
    Bound, ColumnCapabilities, ColumnInfo, FilterChoices, ValueCategory, ValueChoice,
};
pub use dataset::Dataset;
pub use error::{Error, Result};
pub use file::{
    xlsx_sheet_names, Export, ExportOptions, ExportProgress, FileInfo, FileMetadata, FileType,
    OpenOptions, ParquetCompression,
};
pub(crate) use query::{predicate, scope, selection};
pub use query::{
    CellMatch, ColSort, ColSpec, Filter, FilterCondition, Find, FindHighlight, QuerySpec, RowBlock,
    MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};
pub use stats::{
    AnalysisCapabilities, AnalysisField, AxisScale, ColumnOverview, Histogram, LengthSummary,
    LengthUnit, NumericStatistics, RangeBounds, RangeHistogram, ScatterAxisKind, ScatterPoint,
    ScatterResult, ValueFrequency, ValueSummary, MAX_SCATTER_POINTS,
};
pub use view::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices, View};
