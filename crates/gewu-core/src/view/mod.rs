//! Immutable filtered and ordered views over shared source data.

use std::{collections::HashSet, sync::Arc};

use arrow::array::ArrayRef;

use crate::{
    col_view::ColView,
    column::Column,
    execution::sort::{codes_match_arrow_order, sort_rows, SortKey},
    row_ids::RowIds,
    ColumnId, ColumnSort, Dataset, Result,
};

mod analysis;
mod column_segment;
mod export;
mod filter;
mod find;
mod selection;

pub use column_segment::{FormattedColumnSegment, SourceRows};
pub(crate) use selection::FilterSelection;

/// Immutable snapshot of filters applied to a [`Dataset`].
/// Owns the selected rows and the information needed to derive facets.
/// Provides column analysis, filter metadata, and sorting.
pub struct FilteredView {
    dataset: Arc<Dataset>,
    rows: RowIds,
    /// Filter bitmaps behind `rows`; empty for an unfiltered view.
    selection: FilterSelection,
}

/// Ordered row projection over an immutable [`Dataset`].
/// Owns display row order independently of [`FilteredView`] and provides row
/// formatting, cell Find, and export.
pub struct OrderedView {
    dataset: Arc<Dataset>,
    rows: RowIds,
}

impl FilteredView {
    pub(crate) fn all(dataset: Arc<Dataset>) -> Arc<Self> {
        let n_rows = dataset.n_rows();
        Arc::new(Self {
            dataset,
            rows: RowIds::All(n_rows),
            selection: FilterSelection::default(),
        })
    }

    /// A view of the rows selected by `selection`'s bitmaps, materializing their row IDs once.
    pub(crate) fn selected(dataset: Arc<Dataset>, selection: FilterSelection) -> Arc<Self> {
        let Some(mask) = selection.mask() else {
            return Self::all(dataset);
        };
        let rows = RowIds::Selected(FilterSelection::row_ids(mask).into());
        Arc::new(Self {
            dataset,
            rows,
            selection,
        })
    }

    /// Derive a view of the source rows matching all filters in this snapshot
    /// except the one on `column`. The result may include rows absent from `self`.
    /// Returns the same view if `column` has no filter. Calling `facet` on the
    /// derived view removes another filter without restoring the first.
    pub fn facet(self: &Arc<Self>, column: ColumnId) -> Result<Arc<Self>> {
        self.dataset.column(column)?;
        if !self.selection.constrains(column) {
            return Ok(self.clone());
        }
        Ok(Self::selected(
            self.dataset.clone(),
            self.selection.without(column),
        ))
    }

    /// Selection bitmap over all source rows, or `None` when every row is selected.
    pub(crate) fn selection_mask(&self) -> Option<&arrow::buffer::BooleanBuffer> {
        self.selection.mask()
    }

    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    /// Produce an [`OrderedView`] preserving this view's row order in O(1).
    pub fn unsorted(&self) -> Arc<OrderedView> {
        Arc::new(OrderedView {
            dataset: self.dataset.clone(),
            rows: self.rows.clone(),
        })
    }

    /// Sort this view's rows independently, leaving `self` unchanged.
    /// Empty sort keys return [`Self::unsorted`] sharing the same row selection.
    pub fn sort(&self, sorts: &[ColumnSort]) -> Result<Arc<OrderedView>> {
        let mut seen = HashSet::with_capacity(sorts.len());
        for sort in sorts {
            self.dataset.column(sort.column)?;
            if !seen.insert(sort.column) {
                return Err(crate::ColumnError::Duplicate {
                    operation: crate::ColumnOperation::Sort,
                    column: sort.column,
                }
                .into());
            }
        }
        if sorts.is_empty() {
            return Ok(self.unsorted());
        }

        let mut keys = Vec::with_capacity(sorts.len());
        for sort in sorts {
            let column = self.column(sort.column)?;
            let codes = if codes_match_arrow_order(column.array().data_type()) {
                column.dictionary()?.map(|dictionary| dictionary.as_ref())
            } else {
                None
            };
            keys.push(SortKey {
                array: column.array(),
                descending: sort.descending,
                codes,
            });
        }
        let sorted = sort_rows(self.rows.as_slice(), &keys)?;
        Ok(Arc::new(OrderedView {
            dataset: self.dataset.clone(),
            rows: RowIds::Selected(sorted.into()),
        }))
    }

    pub(crate) fn column(&self, column: ColumnId) -> Result<&Column, crate::ColumnError> {
        self.dataset.column(column)
    }

    pub(crate) fn column_array(&self, column: ColumnId) -> Result<&ArrayRef, crate::ColumnError> {
        Ok(self.column(column)?.array())
    }

    pub(crate) fn col(&self, column: ColumnId) -> Result<ColView<'_>, crate::ColumnError> {
        Ok(ColView::new(self.column(column)?, self.rows.as_slice())
            .with_mask(self.selection.mask()))
    }
}

impl OrderedView {
    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    pub(crate) fn column(&self, column: ColumnId) -> Result<&Column, crate::ColumnError> {
        self.dataset.column(column)
    }

    pub(crate) fn column_array(&self, column: ColumnId) -> Result<&ArrayRef, crate::ColumnError> {
        Ok(self.column(column)?.array())
    }
}

#[cfg(test)]
mod tests;
