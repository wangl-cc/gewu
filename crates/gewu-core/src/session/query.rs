//! View evaluation, sorting and visible row formatting.

use std::sync::Arc;

use arrow::{
    array::{make_comparator, ArrayRef},
    compute::SortOptions,
};

use super::Session;
use crate::{
    column::Column,
    dataset::format,
    engine::CancelToken,
    predicate::CHUNK,
    query::sort_rows,
    scope::{RowScope, Selection},
    Filter, Result, RowBlock, ViewInfo, ViewState,
};

impl Session {
    pub fn current_view(&self) -> ViewInfo {
        self.views.current().info()
    }

    /// Cancel any `apply_view` still running.
    pub fn cancel_pending(&self) {
        self.views.cancel_pending();
    }

    /// Compute and switch to a new view. A newer call cancels this one, in
    /// which case it returns [`crate::Error::Cancelled`] and the old view stays.
    pub fn apply_view(&self, state: &ViewState) -> Result<ViewInfo> {
        let token = self.views.begin();
        let mut rows = self.select(&state.filters, None, &token)?;
        token.check()?;
        if !state.sort.is_empty() {
            rows = self.sort(rows, state, &token)?;
        }
        self.views.commit(&token, rows)
    }

    /// Formatted cells for display rows `start..start+count` of view
    /// `version`. Rows past the end are left out.
    pub fn rows(&self, version: u64, start: u64, count: u32, columns: &[u32]) -> Result<RowBlock> {
        let view = self.views.snapshot(version)?;
        for &c in columns {
            self.column(c)?;
        }

        let len = view.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let source_rows = &view.rows[start as usize..end as usize];

        let arrays: Vec<&ArrayRef> = columns
            .iter()
            .map(|&c| &self.dataset.columns[c as usize])
            .collect();
        let formatters = arrays
            .iter()
            .map(|a| Column::new(a).formatter())
            .collect::<Result<Vec<_>>>()?;

        let mut cells = Vec::with_capacity(source_rows.len() * columns.len());
        for &r in source_rows {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(a.as_ref(), f, r as usize));
            }
        }

        Ok(RowBlock {
            version,
            start,
            n_rows: source_rows.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: source_rows.iter().map(|&r| r as u64).collect(),
        })
    }

    /// Resolve which rows a facet counts.
    pub(super) fn rows_in(&self, scope: RowScope<'_>) -> Result<Selection> {
        let total_rows = self.dataset.n_rows;
        Ok(match scope {
            RowScope::All => Selection::All(total_rows),
            // A view keeps each row at most once, so a full-length view is
            // every row, and counting in file order is faster.
            RowScope::Rows(rows) if rows.len() == total_rows => Selection::All(total_rows),
            RowScope::Rows(rows) => Selection::Rows(rows),
            RowScope::Facet { state, except } => {
                self.column(except)?;
                let rows = self.select(&state.filters, Some(except), &CancelToken::never())?;
                Selection::Rows(Arc::new(rows))
            }
        })
    }

    /// Prepare column predicates once, then combine AND/OR in bounded batches.
    /// Masks are reused across columns and batches, not retained per column.
    /// Filters are reordered by estimated cost to maximize batch-level short-circuiting.
    fn select(
        &self,
        filters: &std::collections::HashMap<u32, Vec<Filter>>,
        skip: Option<u32>,
        token: &CancelToken,
    ) -> Result<Vec<u32>> {
        token.check()?;
        let total_rows = self.dataset.n_rows;
        let mut active_filters: Vec<(u32, &Filter)> = Vec::new();
        for (&col, col_filters) in filters {
            if Some(col) == skip {
                continue;
            }
            for f in col_filters {
                active_filters.push((col, f));
            }
        }
        if active_filters.is_empty() {
            let rows = (0..total_rows as u32).collect();
            token.check()?;
            return Ok(rows);
        }

        // Query optimization:
        // Execute cheap, highly selective filters (null checks, indexed values, range bounds)
        // before expensive string or substring scans. In bounded batch execution, this zeroes
        // out matching rows early and triggers `break` before evaluating costlier predicates.
        active_filters.sort_by_key(|(_, f)| f.estimated_cost());

        let prepared_filters = active_filters
            .iter()
            .map(|&(col, f)| self.prepare_filter(col, f))
            .collect::<Result<Vec<_>>>()?;

        let batch_len = total_rows.min(CHUNK);
        let mut selected = vec![true; batch_len];
        let mut scratch = vec![false; batch_len];
        let mut rows = Vec::new();
        for start in (0..total_rows).step_by(CHUNK) {
            token.check()?;
            let end = (start + CHUNK).min(total_rows);
            let range = start..end;
            let selected = &mut selected[..range.len()];
            let scratch = &mut scratch[..range.len()];
            selected.fill(true);
            for test in &prepared_filters {
                test.intersect(range.clone(), selected, scratch, token)?;
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
        token.check()?;
        Ok(rows)
    }

    fn sort(&self, rows: Vec<u32>, state: &ViewState, token: &CancelToken) -> Result<Vec<u32>> {
        let mut comparators = Vec::with_capacity(state.sort.len());
        for key in &state.sort {
            token.check()?;
            let array = self.column(key.column)?;
            comparators.push(make_comparator(
                array.as_ref(),
                array.as_ref(),
                SortOptions {
                    descending: key.descending,
                    // Nulls always at the bottom, like spreadsheets.
                    nulls_first: false,
                },
            )?);
        }
        let primitive = if state.sort.len() == 1 {
            let key = &state.sort[0];
            let array = self.column(key.column)?;
            array
                .data_type()
                .is_primitive()
                .then_some((array, SortOptions {
                    descending: key.descending,
                    nulls_first: false,
                }))
        } else {
            None
        };
        sort_rows(rows, &comparators, primitive, || token.check())
    }
}
