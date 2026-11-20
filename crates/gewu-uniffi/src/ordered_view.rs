//! Immutable ordered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{ColumnId, ColumnText, FileType, FindSession, GewuError, GewuExport};

#[derive(uniffi::Object)]
pub struct GewuOrderedView {
    pub(crate) inner: Arc<core::OrderedView>,
}

#[uniffi::export]
impl GewuOrderedView {
    pub fn row_count(&self) -> u64 {
        self.inner.row_count()
    }

    /// Format one column over display rows `start..start + count`,
    /// clipped to the available rows.
    pub fn format_column(
        &self,
        column: ColumnId,
        start: u64,
        count: u32,
    ) -> Result<ColumnText, GewuError> {
        Ok(self
            .inner
            .format_column(column.into(), start, count)?
            .into())
    }

    pub fn find(
        &self,
        query: String,
        columns: Vec<ColumnId>,
    ) -> Result<Arc<FindSession>, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(Arc::new(FindSession {
            inner: self.inner.find(&query, &columns)?,
        }))
    }

    /// A single-use export of every row of this view.
    pub fn export(
        &self,
        columns: Vec<ColumnId>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(Arc::new(self.inner.export(&columns, format.into())?.into()))
    }

    /// A single-use export of the display rows in `ranges`, in order, such as a
    /// copied selection.
    pub fn export_rows(
        &self,
        ranges: Vec<RowRange>,
        columns: Vec<ColumnId>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        let ranges: Vec<_> = ranges.iter().map(|range| range.start..range.end).collect();
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(Arc::new(
            self.inner
                .export_rows(&ranges, &columns, format.into())?
                .into(),
        ))
    }
}

/// Display rows `start..end`.
#[derive(uniffi::Record)]
pub struct RowRange {
    pub start: u64,
    pub end: u64,
}
