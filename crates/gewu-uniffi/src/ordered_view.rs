//! Immutable ordered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{ColumnId, ColumnText, FileType, FindSession, GewuError, GewuExport, RowIndices};

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
    ) -> Result<Arc<ColumnText>, GewuError> {
        Ok(Arc::new(
            self.inner
                .format_column(column.into(), start, count)?
                .into(),
        ))
    }

    /// Source row indices for display rows `start..start + count`, clipped
    /// to the available rows.
    pub fn source_rows(&self, start: u64, count: u32) -> Result<Arc<RowIndices>, GewuError> {
        Ok(Arc::new(self.inner.source_rows(start, count).try_into()?))
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

    pub fn export(
        &self,
        columns: Vec<ColumnId>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(Arc::new(self.inner.export(&columns, format.into())?.into()))
    }

    pub fn nested_export_columns(&self, columns: Vec<ColumnId>) -> Result<Vec<String>, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(self.inner.nested_export_columns(&columns)?)
    }
}
