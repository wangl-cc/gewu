//! UniFFI bindings for gewu-core.
//!
//! All methods are synchronous. Callers schedule scans off the UI thread.
//! Dataset and View handles share immutable data.

mod analysis;
mod cache;
mod column;
mod data;
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

pub use data::{ColumnText, ExportedBytes, GewuBytes, GewuSourceRows};
pub use dataset::{DatasetInfo, GewuDataset, OpenOptions};
pub use error::{
    AnalysisErrorKind, AnalysisOperation, ExportErrorKind, FileErrorKind, FilterErrorKind,
    FindErrorKind, GewuError, IoOperation, LimitErrorKind, ValueParseError,
};
pub use export::GewuExport;
pub use file::{xlsx_sheet_names, FileMetadata, KeyValue};
pub use filtered_view::GewuFilteredView;
pub use find::FindSession;
pub use ordered_view::{GewuOrderedView, RowRange};
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult};

uniffi::setup_scaffolding!();
