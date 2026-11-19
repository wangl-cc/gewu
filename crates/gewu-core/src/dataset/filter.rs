//! Filter validation and execution over the full dataset.

use std::{collections::HashSet, sync::Arc};

use super::Dataset;
use crate::{view::FilterSelection, ColumnFilter, FilteredView, Result};

impl Dataset {
    /// Filter the full dataset, producing an independent [`FilteredView`].
    /// Empty filter lists return [`Self::unfiltered`] with zero row-ID allocation.
    pub fn filter(self: &Arc<Self>, filters: &[ColumnFilter]) -> Result<Arc<FilteredView>> {
        let mut seen = HashSet::with_capacity(filters.len());
        for f in filters {
            self.column(f.column)?;
            if !seen.insert(f.column) {
                return Err(crate::ColumnError::Duplicate {
                    operation: crate::ColumnOperation::Filter,
                    column: f.column,
                }
                .into());
            }
        }
        if filters.is_empty() {
            return Ok(self.unfiltered());
        }

        // Each condition yields a bitmap over every source row, so a filter
        // editor can later combine the other columns' bitmaps without
        // re-running predicates. Row IDs are materialized once, from the
        // intersection.
        let n_rows = self.n_rows;
        let conditions = filters
            .iter()
            .map(|f| {
                let predicate = self.column(f.column)?.compile_filter(&f.filter)?;
                Ok((f.column, predicate.select(0..n_rows)))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(FilteredView::selected(
            self.clone(),
            FilterSelection::new(conditions),
        ))
    }
}
