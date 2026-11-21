//! The rows a table shows: a filtered view in display order.
use std::sync::Arc;

use gewu_core::{self as core, ColumnFilter, ColumnId, ColumnSort, FileType};

use crate::{ColumnText, GewuError, GewuExport, GewuFilteredView, GewuFind};

/// The rows of a [`GewuFilteredView`] ordered by `sorts`. Immutable: reads,
/// Find and exports capture exactly these rows.
#[derive(uniffi::Object)]
pub struct GewuOrderedView {
    pub(crate) inner: Arc<core::OrderedView>,
    pub(crate) filtered: Arc<GewuFilteredView>,
    pub(crate) sorts: Vec<ColumnSort>,
}

#[uniffi::export]
impl GewuOrderedView {
    /// The rows passing `filters` ordered by `sorts`, reusing what is
    /// unchanged: this view when both match, and its filtered rows with
    /// their analyses when only the order differs. Filters on different
    /// columns combine with AND; sorts are by priority.
    pub fn apply(
        self: Arc<Self>,
        filters: Vec<ColumnFilter>,
        sorts: Vec<ColumnSort>,
    ) -> Result<Arc<Self>, GewuError> {
        if filters == self.filtered.filters {
            if sorts == self.sorts {
                return Ok(self);
            }
            return self.filtered.sorted(sorts);
        }
        GewuFilteredView::new(Arc::clone(&self.filtered.dataset), filters)?.sorted(sorts)
    }

    /// These rows in file order, which serve every analysis.
    pub fn filtered(&self) -> Arc<GewuFilteredView> {
        Arc::clone(&self.filtered)
    }

    /// Whether `other` is this very view, such as when [`Self::apply`] reused
    /// it. Each Swift wrapper is a separate reference, so Swift cannot tell
    /// by itself.
    pub fn is_same_view(&self, other: Arc<Self>) -> bool {
        std::ptr::eq(self, Arc::as_ptr(&other))
    }

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
        Ok(self.inner.format_column(column, start, count)?.into())
    }

    pub fn find(&self, query: String, columns: Vec<ColumnId>) -> Result<Arc<GewuFind>, GewuError> {
        Ok(Arc::new(GewuFind {
            inner: self.inner.find(&query, &columns)?,
        }))
    }

    /// A single-use export of every row of this view.
    pub fn export(
        &self,
        columns: Vec<ColumnId>,
        format: FileType,
    ) -> Result<Arc<GewuExport>, GewuError> {
        Ok(Arc::new(self.inner.export(&columns, format)?.into()))
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
        Ok(Arc::new(
            self.inner.export_rows(&ranges, &columns, format)?.into(),
        ))
    }
}

/// Display rows `start..end`.
#[derive(uniffi::Record)]
pub struct RowRange {
    pub start: u64,
    pub end: u64,
}
