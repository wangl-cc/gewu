//! View evaluation, sorting and visible row formatting.

use std::sync::Arc;

use arrow::{
    array::{make_comparator, ArrayRef},
    compute::SortOptions,
};

use super::View;
use crate::{
    format, predicate::CHUNK, query::sort_rows, Cancellation, Filter, QuerySpec, Result, RowBlock,
};

impl View {
    fn validate_query(&self, spec: &QuerySpec) -> Result<()> {
        let mut columns = std::collections::HashSet::new();
        let mut priorities = std::collections::HashSet::new();
        for c in &spec.columns {
            self.column(c.column)?;
            if !columns.insert(c.column) {
                return Err(crate::Error::InvalidArgument(
                    "duplicate query column".into(),
                ));
            }
            if c.sort.is_some_and(|sort| !priorities.insert(sort.priority)) {
                return Err(crate::Error::InvalidArgument(
                    "duplicate sort priority".into(),
                ));
            }
        }
        Ok(())
    }

    /// Compute a new result from the full dataset, independent of this view's selection.
    pub fn query(&self, spec: &QuerySpec, cancellation: &Cancellation) -> Result<Arc<Self>> {
        cancellation.check()?;
        self.validate_query(spec)?;
        let mut rows = self.select(spec, cancellation)?;
        if !spec.sorts().is_empty() {
            rows = self.sort(rows, spec, cancellation)?;
        }
        let view = Arc::new(Self {
            dataset: self.dataset.clone(),
            columns: spec.columns.iter().map(|column| column.column).collect(),
            rows: rows.into(),
        });
        cancellation.check()?;
        Ok(view)
    }

    /// Formatted cells at display positions, in the requested source-column order.
    pub fn rows(&self, start: u64, count: u32, columns: &[u32]) -> Result<RowBlock> {
        for &c in columns {
            self.column(c)?;
        }

        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let source_rows = &self.rows[start as usize..end as usize];

        let arrays: Vec<&ArrayRef> = columns
            .iter()
            .map(|&c| self.dataset.columns[c as usize].array())
            .collect();
        let formatters = columns
            .iter()
            .map(|&c| format::formatter(self.dataset.columns[c as usize].array().as_ref()))
            .collect::<Result<Vec<_>>>()?;

        let mut cells = Vec::with_capacity(source_rows.len() * columns.len());
        for &r in source_rows {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(a.as_ref(), f, r as usize));
            }
        }

        Ok(RowBlock {
            start,
            n_rows: source_rows.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: source_rows.iter().map(|&r| r as u64).collect(),
        })
    }

    /// Compile column predicates once, then intersect their matches in bounded batches.
    /// Masks are reused across columns and batches, not retained per column.
    /// Filters are reordered by estimated cost to maximize batch-level short-circuiting.
    fn select(&self, spec: &QuerySpec, token: &Cancellation) -> Result<Vec<u32>> {
        token.check()?;
        let total_rows = self.dataset.n_rows;
        let mut active_filters: Vec<(u32, &Filter)> = Vec::new();
        for col in &spec.columns {
            if let Some(filter) = &col.filter {
                active_filters.push((col.column, filter));
            }
        }
        if active_filters.is_empty() {
            let mut rows = Vec::with_capacity(total_rows);
            for row in 0..total_rows as u32 {
                token.check_at(row as usize)?;
                rows.push(row);
            }
            token.check()?;
            return Ok(rows);
        }

        // Query optimization:
        // Execute cheap, highly selective filters (null checks, indexed values, range bounds)
        // before expensive string or substring scans. In bounded batch execution, this zeroes
        // out matching rows early and triggers `break` before evaluating costlier predicates.
        active_filters.sort_by_key(|(_, f)| f.estimated_cost());

        let predicates = active_filters
            .iter()
            .map(|&(col, f)| self.column(col)?.compile_filter(f, token))
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
            for test in &predicates {
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

    fn sort(&self, rows: Vec<u32>, spec: &QuerySpec, token: &Cancellation) -> Result<Vec<u32>> {
        let sorts = spec.sorts();
        let mut comparators = Vec::with_capacity(sorts.len());
        for key in &sorts {
            token.check()?;
            let array = self.column_array(key.column)?;
            comparators.push(make_comparator(
                array.as_ref(),
                array.as_ref(),
                SortOptions {
                    descending: key.sort.unwrap().descending,
                    // Nulls always at the bottom, like spreadsheets.
                    nulls_first: false,
                },
            )?);
        }
        let primitive = if sorts.len() == 1 {
            let key = sorts[0];
            let array = self.column_array(key.column)?;
            array
                .data_type()
                .is_primitive()
                .then_some((array, SortOptions {
                    descending: key.sort.unwrap().descending,
                    nulls_first: false,
                }))
        } else {
            None
        };
        sort_rows(rows, &comparators, primitive, || token.check())
    }
}
