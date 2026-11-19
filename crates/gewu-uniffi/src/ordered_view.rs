//! Immutable ordered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{
    ColumnBatch, ColumnId, FileType, FindSession, FormattedColumnText, GewuError, GewuExport,
    RowSpan, SegmentBatch,
};

#[derive(uniffi::Object)]
pub struct GewuOrderedView {
    pub(crate) inner: Arc<core::OrderedView>,
}

#[uniffi::export]
impl GewuOrderedView {
    pub fn row_count(&self) -> u64 {
        self.inner.row_count()
    }

    /// Format one column at display rows `start..start + count`, clipped to
    /// the available rows.
    pub fn format_column(
        &self,
        column: ColumnId,
        start: u64,
        count: u32,
    ) -> Result<FormattedColumnText, GewuError> {
        self.inner
            .format_column(column.into(), start, count)?
            .try_into()
    }

    pub fn format_columns(
        &self,
        columns: Vec<ColumnId>,
        start: u64,
        count: u32,
    ) -> Result<FormattedColumnText, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        self.inner
            .format_columns(&columns, start, count)?
            .try_into()
    }

    pub fn source_rows(&self, start: u64, count: u32) -> Vec<u64> {
        self.inner.source_rows(start, count)
    }

    /// Read a viewport's source rows and column segments in one call. The App
    /// decides which segments to read and cache; Core formats each one. An
    /// unknown column fails the whole call without a partial result.
    pub fn read_segments(
        &self,
        rows: Vec<RowSpan>,
        batches: Vec<ColumnBatch>,
    ) -> Result<SegmentBatch, GewuError> {
        let batches = batches
            .into_iter()
            .map(|batch| self.format_columns(batch.columns, batch.start, batch.count))
            .collect::<Result<_, _>>()?;
        Ok(SegmentBatch {
            source_rows: rows
                .iter()
                .map(|span| self.inner.source_rows(span.start, span.count))
                .collect(),
            batches,
        })
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
