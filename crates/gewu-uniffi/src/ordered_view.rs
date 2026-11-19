//! Immutable ordered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{
    ColumnId, ColumnSpan, FileType, FindSession, FormattedColumnSegment, GewuError, GewuExport,
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
    ) -> Result<FormattedColumnSegment, GewuError> {
        self.inner
            .format_column(column.into(), start, count)?
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
        segments: Vec<ColumnSpan>,
    ) -> Result<SegmentBatch, GewuError> {
        let segments = segments
            .iter()
            .map(|span| self.format_column(span.column, span.start, span.count))
            .collect::<Result<_, _>>()?;
        Ok(SegmentBatch {
            source_rows: rows
                .iter()
                .map(|span| self.inner.source_rows(span.start, span.count))
                .collect(),
            segments,
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

    /// Export only the given display-row ranges, in order, such as a table
    /// selection. Ranges are clipped to the available rows.
    pub fn export_rows(
        &self,
        ranges: Vec<RowSpan>,
        columns: Vec<ColumnId>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        let ranges: Vec<_> = ranges
            .iter()
            .map(|span| span.start..span.start.saturating_add(u64::from(span.count)))
            .collect();
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(Arc::new(
            self.inner
                .export_rows(&ranges, &columns, format.into())?
                .into(),
        ))
    }

    pub fn nested_export_columns(&self, columns: Vec<ColumnId>) -> Result<Vec<String>, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(self.inner.nested_export_columns(&columns)?)
    }
}
