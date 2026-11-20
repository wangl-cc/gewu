//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. Callers schedule scans off the UI thread.
//! Dataset and View handles share immutable data.

mod analysis;
mod buffer;
mod column;
mod dataset;
mod error;
mod export;
mod file;
mod filter;
mod filtered_view;
mod find;
mod ordered_view;
mod query;
mod scale;
mod scatter;
#[cfg(test)]
mod tests;

pub use analysis::{
    AnalysisCapabilities, AnalysisField, Histogram, LengthSummary, LengthUnit, NumericStatistics,
    ValueFrequency, ValueSummary,
};
pub use buffer::{BufferView, ColumnText, RowIndices};
pub use column::{ColumnCapabilities, ColumnId, ColumnInfo, ValueCategory};
pub use dataset::{open_dataset, DatasetInfo, GewuDataset, OpenOptions};
pub use error::{
    AnalysisErrorKind, AnalysisOperation, ExportErrorKind, FileErrorKind, FilterErrorKind,
    FindErrorKind, GewuError, IoOperation, LimitErrorKind, ValueParseError,
};
pub use export::{ExportOptions, ExportProgress, GewuExport, ParquetCompression};
pub use file::{xlsx_sheet_names, FileMetadata, FileType, KeyValue};
pub use filter::{
    Filter, FilterChoices, FilterCondition, RangeBound, RangeBounds, RangeHistogram, TextMatch,
    ValueChoice,
};
pub use filtered_view::GewuFilteredView;
pub use find::{find_limits, CellMatch, ColumnHighlight, FindHighlight, FindLimits, FindSession};
pub use ordered_view::GewuOrderedView;
pub use query::{ColumnFilter, ColumnSort};
pub use scale::AxisScale;
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult};

uniffi::setup_scaffolding!();
