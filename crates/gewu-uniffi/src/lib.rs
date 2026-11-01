//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. The app calls scans and I/O from background
//! tasks. A newer `apply_view` cancels an older view request inside the core;
//! analysis and export jobs have independent cancellation handles. Snapshot
//! preparation, capability metadata and visible row blocks are UI-thread APIs.

mod analysis;
mod error;
mod export;
mod file;
mod filter;
mod find;
mod scale;
mod scatter;
mod session;
mod text_choices;
mod view;

pub use analysis::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, ColumnAnalysis, ColumnSummary,
    DistinctValue, Histogram, LengthSummary, LengthUnit, NumericStatistics, ValueSummary,
};
pub use error::GewuError;
pub use export::{ExportOptions, ExportProgress, GewuExport, ParquetCompression};
pub use file::{
    xlsx_sheet_names, ColumnCapabilities, ColumnInfo, FileInfo, FileType, KeyValue, LogicalType,
    TimeUnit,
};
pub use filter::{
    Bound, Filter, FilterEditor, FilterSpec, RangeBounds, RangeHistogram, TextFilterMode,
};
pub use find::{find_limits, CellMatch, FindHighlight, FindLimits, FindOperation, FindSession};
pub use scale::AxisScale;
pub use scatter::{ScatterAnalysis, ScatterPoint, ScatterResult};
pub use session::GewuSession;
pub use text_choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};
pub use view::{QuerySpec, RowBlock, SortKey, ViewInfo};

uniffi::setup_scaffolding!();
