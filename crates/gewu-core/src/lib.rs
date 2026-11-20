//! Data engine for gewu, a read-only table viewer.
//!
//! The Mac app shares immutable [`Dataset`], [`FilteredView`], and [`OrderedView`]
//! objects through UniFFI. Callers own query conditions and choose which result
//! to display. Sorting and filtering never move data; they only produce a new
//! list of row numbers.

mod col_view;
mod column;
mod dataset;
mod error;
mod execution;
mod file;
mod find;
pub(crate) mod format;
pub(crate) mod query;
mod row_ids;
mod scalar;
mod stats;
mod types;
mod view;

pub use column::{
    ColumnCapabilities, ColumnError, ColumnId, ColumnInfo, ColumnOperation, FilterChoices,
    FilterError, FilterKind, ValueCategory, ValueChoice, ValueParseError,
};
pub use dataset::{Dataset, DatasetInfo};
pub use error::{Error, InternalError, IoError, IoOperation, Result};
pub use file::{
    xlsx_sheet_names, CodecError, Export, ExportError, ExportOptions, ExportProgress, FileError,
    FileMetadata, FileOperation, FileType, OpenOptions, ParquetCompression,
};
pub use find::{
    CellMatch, ColumnHighlight, Find, FindError, FindHighlight, MAX_FIND_HIGHLIGHT_ROWS,
    MAX_FIND_PAGE_SIZE,
};
pub use query::{ColumnFilter, ColumnSort, Filter, FilterCondition, TextMatch};
pub use scalar::BoundError;
pub use stats::{
    AnalysisCapabilities, AnalysisError, AnalysisField, AnalysisOperation, AxisScale, Histogram,
    LengthSummary, LengthUnit, NumericStatistics, RangeBound, RangeBounds, RangeHistogram,
    ScatterAxisKind, ScatterPoint, ScatterResult, ValueFrequency, ValueSummary, MAX_SCATTER_POINTS,
};
pub use view::{FilteredView, FormattedColumnSegment, OrderedView};
