//! Data engine for gewu, a read-only table viewer.
//!
//! The UI (AppKit, CLI, MCP) talks to a [`Document`]. A document owns the
//! decoded data and a current view: the list of source rows to show, in
//! display order. Sorting and filtering never move data; they only produce a
//! new list of row numbers.

mod analysis;
mod column;
mod document;
mod engine;
mod error;
mod export;
mod file;
mod filter;
mod format;
mod kind;
mod predicate;
mod scale;
mod scatter;
mod scope;
mod text_choices;
mod value_index;
mod view;

pub use analysis::{
    AnalysisData, AnalysisField, AnalysisResult, AnalysisSection, ColumnAnalysis, ColumnSummary,
    LengthSummary, LengthUnit, NumericStatistics, ValueSummary,
};
pub use column::{
    bound::Bound,
    histogram::{Histogram, RangeBounds, RangeHistogram},
    ColumnCapabilities, ColumnInfo,
};
pub use document::Document;
pub use error::{Error, Result};
pub use export::{Export, ExportProgress};
pub use file::{
    xlsx_sheet_names, ExportOptions, FileInfo, FileMetadata, FileType, OpenOptions,
    ParquetCompression,
};
pub use filter::{Filter, FilterEditor, FilterSpec, TextFilterMode};
pub use kind::DataKind;
pub use scale::AxisScale;
pub use scatter::{ScatterAnalysis, ScatterPoint, ScatterResult, MAX_SCATTER_POINTS};
pub use text_choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};
pub use value_index::{DistinctResult, DistinctValue};
pub use view::{RowBlock, SortKey, ViewInfo, ViewState};
