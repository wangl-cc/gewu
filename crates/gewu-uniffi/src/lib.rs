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

pub use data::{ColumnText, GewuBytes, GewuExportedBytes, GewuSourceRows};
pub use dataset::{DatasetInfo, GewuDataset, OpenOptions};
pub use error::{
    AnalysisErrorKind, ExportErrorKind, FileErrorKind, FilterErrorKind, FindErrorKind, GewuError,
    LimitErrorKind,
};
pub use export::GewuExport;
pub use file::{xlsx_sheet_names, FileMetadata, KeyValue};
pub use filtered_view::GewuFilteredView;
pub use find::GewuFind;
pub use ordered_view::{GewuOrderedView, RowRange};
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult};

uniffi::setup_scaffolding!();
