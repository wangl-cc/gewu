//! Data engine for gewu, a read-only table viewer.
//!
//! The UI (AppKit, CLI, MCP) talks to a [`Document`]. A document owns the
//! decoded data and a current [`View`]: the list of source rows to show, in
//! display order. Sorting and filtering never move data; they only produce a
//! new list of row numbers.

mod column;
mod document;
mod engine;
mod error;
mod export;
mod file;
mod format;
mod kind;
mod numeric;
mod predicate;
mod scope;
mod value_index;
mod view;

pub use column::{bound::Bound, ColumnCapabilities};
pub use document::Document;
pub use engine::CancelToken;
pub use error::{Error, Result};
pub use export::{Export, ExportOptions, ExportProgress};
pub use file::{FileMetadata, FileType, OpenOptions};
pub use kind::DataKind;
pub use view::{
    ColumnInfo, ColumnSummary, DistinctResult, DistinctValue, FileInfo, Filter, FilterEditor,
    FilterSpec, Histogram, RangeBounds, RangeHistogram, RowBlock, SortKey, ViewInfo, ViewState,
};
