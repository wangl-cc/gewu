//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. The app calls scans and I/O from background
//! tasks. A newer `prepare_view` cancels an older view request inside the core;
//! column analyses use caller-owned cancellation, and exports have independent handles. Snapshot
//! preparation, capability metadata and visible row blocks are UI-thread APIs.

mod analysis;
mod cancellation;
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
    AnalysisField, AnalysisSection, ColumnOverview, Histogram, LengthSummary, LengthUnit,
    NumericStatistics, ValueFrequency, ValueSummary,
};
pub use cancellation::Cancellation;
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
pub use scatter::{ScatterAnalysis, ScatterAxisKind, ScatterPoint, ScatterResult};
pub use session::GewuSession;
pub use text_choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};
pub use view::{ColSort, ColSpec, QuerySpec, QueryState, RowBlock, ViewInfo, ViewOperation};

uniffi::setup_scaffolding!();
