//! Filter validation and execution over the full dataset.
//!
//! Each condition compiles once to a bitmap-only selector and is evaluated
//! over the whole source range into a packed selection bitmap. Null policy,
//! exclusion and alternatives combine bitmaps with bitwise operations. Leaf
//! predicates (ranges, scalars, text) are shared with Find: native range,
//! dictionary-code and string leaves compare 64 rows per bitmap word. The
//! filtered view keeps one bitmap per condition, so a filter editor's facet is
//! the intersection of the others, and the intersection of all of them is
//! materialized once as source row IDs.

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

        // Reject invalid conditions before evaluating any full-range selection.
        let predicates = filters
            .iter()
            .map(|f| Ok((f.column, self.column(f.column)?.compile_filter(&f.filter)?)))
            .collect::<Result<Vec<_>>>()?;

        // Keep each condition's full-range bitmap so facets can combine the
        // other columns' bitmaps without re-running predicates.
        let conditions = predicates
            .into_iter()
            .map(|(column, predicate)| (column, predicate.select(0..self.n_rows)))
            .collect();
        Ok(FilteredView::selected(
            self.clone(),
            FilterSelection::new(conditions),
        ))
    }
}
