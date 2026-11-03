//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. Callers schedule scans off the UI thread and
//! own cancellation signals. Dataset and View handles share immutable data.

mod analysis;
mod cancellation;
mod dataset;
mod error;
mod export;
mod file;
mod filter;
mod find;
mod scale;
mod scatter;
#[cfg(test)]
mod tests;
mod text_choices;
mod view;
mod view_handle;

pub use analysis::{
    AnalysisCapabilities, AnalysisField, Histogram, LengthSummary, LengthUnit, NumericStatistics,
    ValueFrequency, ValueSummary,
};
pub use cancellation::Cancellation;
pub use dataset::GewuDataset;
pub use error::GewuError;
pub use export::{ExportOptions, ExportProgress, GewuExport, ParquetCompression};
pub use file::{
    xlsx_sheet_names, ColumnCapabilities, ColumnInfo, FileInfo, FileType, KeyValue, ValueCategory,
};
pub use filter::{
    Filter, FilterChoices, FilterCondition, RangeBound, RangeBounds, RangeHistogram, ValueChoice,
};
pub use find::{find_limits, CellMatch, FindHighlight, FindLimits, FindSession};
pub use scale::AxisScale;
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult};
pub use text_choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};
pub use view::{ColSort, ColSpec, QuerySpec, RowBlock};
pub use view_handle::GewuView;

uniffi::setup_scaffolding!();
