//! Filter validation and execution over the full dataset.

use std::{collections::HashSet, sync::Arc};

use super::Dataset;
use crate::{execution::predicate::CHUNK, ColumnFilter, Filter, FilteredView, Result};

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

        // Query optimization:
        // Execute cheap, highly selective filters (null checks, indexed values, range bounds)
        // before expensive string or substring scans. In bounded batch execution, this zeroes
        // out matching rows early and triggers `break` before evaluating costlier predicates.
        let mut active: Vec<&ColumnFilter> = filters.iter().collect();
        active.sort_by_key(|f| filter_cost(&f.filter));

        let predicates = active
            .iter()
            .map(|f| self.column(f.column)?.compile_filter(&f.filter))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let total_rows = self.n_rows;
        let batch_len = total_rows.min(CHUNK);
        let mut selected = vec![true; batch_len];
        let mut scratch = vec![false; batch_len];
        let mut rows = Vec::new();
        for start in (0..total_rows).step_by(CHUNK) {
            let end = (start + CHUNK).min(total_rows);
            let range = start..end;
            let selected = &mut selected[..range.len()];
            let scratch = &mut scratch[..range.len()];
            selected.fill(true);
            for test in &predicates {
                test.intersect(range.clone(), selected, scratch);
                if !selected.contains(&true) {
                    break;
                }
            }
            rows.extend(
                range
                    .zip(selected)
                    .filter_map(|(r, hit)| (*hit).then_some(r as u32)),
            );
        }

        Ok(FilteredView::selected(self.clone(), rows))
    }
}

fn filter_cost(filter: &Filter) -> u8 {
    match &filter.condition {
        crate::FilterCondition::Exact { values, .. } if values.is_empty() => 0,
        crate::FilterCondition::Exact { .. } => 1,
        crate::FilterCondition::Range { .. } => 2,
        crate::FilterCondition::Text { .. } => 3,
    }
}
