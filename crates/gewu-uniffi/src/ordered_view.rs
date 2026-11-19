//! Immutable ordered view handles over shared source data.
use std::sync::Arc;

use gewu_core as core;

use crate::{
    CellText, CellTile, ColumnId, FileType, FindSession, GewuError, GewuExport, RowSpan, TileBatch,
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

    pub fn cell_text(
        &self,
        start: u64,
        count: u32,
        columns: Vec<ColumnId>,
    ) -> Result<CellText, GewuError> {
        let columns: Vec<core::ColumnId> = columns.into_iter().map(Into::into).collect();
        Ok(self.inner.cell_text(start, count, &columns)?.into())
    }

    pub fn source_rows(&self, start: u64, count: u32) -> Vec<u64> {
        self.inner.source_rows(start, count)
    }

    /// Read every missing viewport tile in one call. Each cell tile has its own
    /// text buffer, so a long value only enlarges the tile that contains it.
    pub fn read_tiles(
        &self,
        rows: Vec<RowSpan>,
        cells: Vec<CellTile>,
    ) -> Result<TileBatch, GewuError> {
        // Swift decides which tiles to read and cache; Core formats each one.
        let cells = cells
            .iter()
            .map(|tile| {
                let text = self
                    .inner
                    .cell_text(tile.start, tile.count, &[tile.column.into()])?;
                Ok(text.into())
            })
            .collect::<Result<_, GewuError>>()?;
        Ok(TileBatch {
            source_rows: rows
                .iter()
                .map(|span| self.inner.source_rows(span.start, span.count))
                .collect(),
            cells,
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
