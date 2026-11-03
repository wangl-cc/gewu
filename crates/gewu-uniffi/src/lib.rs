//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. Callers schedule scans off the UI thread and
//! own cancellation signals. Dataset and View handles share immutable data.

mod analysis;
mod cancellation;
mod column;
mod dataset;
mod error;
mod export;
mod file;
mod filter;
mod find;
mod query;
mod scale;
mod scatter;
#[cfg(test)]
mod tests;
mod view;
mod view_handle;

pub use analysis::{
    AnalysisCapabilities, AnalysisField, Histogram, LengthSummary, LengthUnit, NumericStatistics,
    ValueFrequency, ValueSummary,
};
pub use cancellation::Cancellation;
pub use column::{ColumnCapabilities, ColumnInfo, ValueCategory};
pub use dataset::{DatasetInfo, GewuDataset};
pub use error::GewuError;
pub use export::{ExportOptions, ExportProgress, GewuExport, ParquetCompression};
pub use file::{xlsx_sheet_names, FileMetadata, FileType, KeyValue};
pub use filter::{
    Filter, FilterChoices, FilterCondition, RangeBound, RangeBounds, RangeHistogram, TextMatch,
    ValueChoice,
};
pub use find::{find_limits, CellMatch, FindHighlight, FindLimits, FindSession};
pub use query::{ColSort, ColSpec, QuerySpec};
pub use scale::AxisScale;
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult};
pub use view::RowBlock;
pub use view_handle::GewuView;

uniffi::setup_scaffolding!();
