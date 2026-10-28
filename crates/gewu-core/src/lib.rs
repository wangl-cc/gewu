//! Data engine for gewu, a read-only table viewer.
//!
//! The UI (AppKit, CLI, MCP) talks to a [`Document`]. A document owns the
//! decoded data and a current [`View`]: the list of source rows to show, in
//! display order. Sorting and filtering never move data; they only produce a
//! new list of row numbers.

mod document;
mod error;
mod format;
mod value_index;
mod view;

pub use document::{CancelToken, Document};
pub use error::{Error, Result};
pub use view::{
    ColumnInfo, ColumnStats, DistinctResult, DistinctValue, FileInfo, Filter, RowBlock, SortKey,
    ViewInfo, ViewState,
};
