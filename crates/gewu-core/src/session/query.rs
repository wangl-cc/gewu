//! View evaluation, sorting and visible row formatting.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use arrow::{
    array::{make_comparator, ArrayRef},
    compute::SortOptions,
};

use super::Session;
use crate::{
    format,
    predicate::CHUNK,
    query::sort_rows,
    scope::{RowScope, Selection},
    Cancellation, Filter, QuerySpec, Result, RowBlock, ViewInfo,
};

/// A single-use view change registered before it is queued for execution.
pub struct ViewOperation {
    session: Session,
    spec: QuerySpec,
    cancellation: Cancellation,
    started: AtomicBool,
}

impl ViewOperation {
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn run(&self) -> Result<ViewInfo> {
        self.cancellation.check()?;
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(crate::Error::InvalidArgument(
                "view operation already started".into(),
            ));
        }
        self.session.execute_view(&self.spec, &self.cancellation)
    }
}

impl Session {
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

    pub fn current_view(&self) -> ViewInfo {
        self.views.current().info()
    }

    /// Read the committed query and its version from one snapshot. A cancelled
    /// UI request may have committed before cancellation reached it.
    pub fn current_query(&self) -> (ViewInfo, QuerySpec) {
        let view = self.views.current();
        (view.info(), view.spec.clone())
    }

    /// Register request order and capture inputs; no row scans occur here.
    pub fn prepare_view(&self, spec: &QuerySpec) -> Result<ViewOperation> {
        self.validate_query(spec)?;
        Ok(ViewOperation {
            session: self.clone(),
            spec: spec.clone(),
            cancellation: self.views.begin(),
            started: AtomicBool::new(false),
        })
    }

    fn execute_view(&self, spec: &QuerySpec, token: &Cancellation) -> Result<ViewInfo> {
        token.check()?;
        let current = self.views.current();
        let rows = if current.spec.same_rows(spec) {
            current.rows.clone()
        } else {
            let mut rows = self.select(spec, None, token)?;
            token.check()?;
            if !spec.sorts().is_empty() {
                rows = self.sort(rows, spec, token)?;
            }
            Arc::new(rows)
        };
        self.views.commit(token, rows, spec.clone())
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
            .map(|&c| &self.dataset.columns[c as usize].array)
            .collect();
        let formatters = columns
            .iter()
            .map(|&c| format::formatter(self.dataset.columns[c as usize].array.as_ref()))
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

    pub(super) fn rows_in(&self, scope: RowScope<'_>, cancel: &Cancellation) -> Result<Selection> {
        cancel.check()?;
        let total_rows = self.dataset.n_rows;
        Ok(match scope {
            RowScope::All => Selection::All(total_rows),
            RowScope::Facet { spec, except } => {
                self.column(except)?;
                self.validate_query(spec)?;
                if !spec
                    .columns
                    .iter()
                    .any(|col| col.column != except && col.filter.is_some())
                {
                    return Ok(Selection::All(total_rows));
                }
                let rows = self.select(spec, Some(except), cancel)?;
                Selection::Rows(Arc::new(rows))
            }
        })
    }

    /// Prepare column predicates once, then combine AND/OR in bounded batches.
    /// Masks are reused across columns and batches, not retained per column.
    /// Filters are reordered by estimated cost to maximize batch-level short-circuiting.
    fn select(
        &self,
        spec: &QuerySpec,
        skip: Option<u32>,
        token: &Cancellation,
    ) -> Result<Vec<u32>> {
        token.check()?;
        let total_rows = self.dataset.n_rows;
        let mut active_filters: Vec<(u32, &Filter)> = Vec::new();
        self.validate_query(spec)?;
        for col in &spec.columns {
            if Some(col.column) != skip {
                if let Some(filter) = &col.filter {
                    active_filters.push((col.column, filter));
                }
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

        let prepared_filters = active_filters
            .iter()
            .map(|&(col, f)| self.column(col)?.prepare_filter(f, token))
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

#[cfg(all(test, feature = "csv"))]
mod tests {
    use super::*;
    use crate::{ColSpec, Error};

    fn session() -> Session {
        let file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
        std::fs::write(file.path(), "a,b,c\n1,2,x\n1,1,y\n2,0,z\n").unwrap();
        Session::open(file.path()).unwrap()
    }

    #[test]
    fn prepared_view_order_survives_reversed_execution_and_late_cancellation() {
        let doc = session();
        let old = doc
            .prepare_view(&QuerySpec::all(3).with_sort(0, true, 0))
            .unwrap();
        let new = doc
            .prepare_view(&QuerySpec::all(3).with_sort(1, false, 0))
            .unwrap();
        let accepted = new.run().unwrap();
        assert!(matches!(old.run(), Err(Error::Cancelled)));
        old.cancel();
        new.cancel();
        assert_eq!(doc.current_view().version, accepted.version);
        assert_eq!(
            doc.rows(accepted.version, 0, 3, &[]).unwrap().source_rows,
            [2, 1, 0]
        );
        let cancelled = doc.prepare_view(&QuerySpec::all(3)).unwrap();
        cancelled.cancel();
        assert!(matches!(cancelled.run(), Err(Error::Cancelled)));
        assert_eq!(doc.current_view().version, accepted.version);
    }

    #[test]
    fn facet_preparation_propagates_cancellation_before_any_result_is_published() {
        let file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
        std::fs::write(
            file.path(),
            format!("number,label\n{}", "1,value\n".repeat(20_000)),
        )
        .unwrap();
        let doc = Session::open(file.path()).unwrap();
        let spec = QuerySpec::all(2).with_filter(0, Filter::range(Some("1".into()), None, false));
        // Entry, facet, select, predicate preparation, then a scan checkpoint.
        assert!(matches!(
            doc.prepare_text_choices(1, Some(&spec), &Cancellation::after_checks(6)),
            Err(Error::Cancelled)
        ));
        assert_eq!(doc.current_view().version, 0);
        let choices = doc
            .prepare_text_choices(1, Some(&spec), &Cancellation::default())
            .unwrap();
        assert_eq!(
            choices
                .page("", None, 1, &Cancellation::default())
                .unwrap()
                .values[0]
                .count,
            20_000
        );
        assert!(matches!(
            doc.filter_distribution(
                0,
                None,
                10,
                crate::AxisScale::Linear,
                &Cancellation::after_checks(6)
            ),
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            doc.validate_filter(
                0,
                &Filter::exact(vec!["1".into()], false, false),
                &Cancellation::after_checks(5)
            ),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn projection_reuses_rows_and_sort_priorities_are_independent() {
        let doc = session();
        let mut spec = QuerySpec::all(3)
            .with_sort(0, false, 10)
            .with_sort(1, false, 20);
        doc.prepare_view(&spec)
            .and_then(|operation| operation.run())
            .unwrap();
        let first = doc.views.current();
        assert_eq!(*first.rows, [1, 0, 2]);
        spec.columns.reverse();
        doc.prepare_view(&spec)
            .and_then(|operation| operation.run())
            .unwrap();
        let reordered = doc.views.current();
        assert!(Arc::ptr_eq(&first.rows, &reordered.rows));
        assert_eq!(
            reordered
                .spec
                .columns
                .iter()
                .map(|c| c.column)
                .collect::<Vec<_>>(),
            [2, 1, 0]
        );
        spec.columns.retain(|c| c.column != 0);
        doc.prepare_view(&spec)
            .and_then(|operation| operation.run())
            .unwrap();
        assert_eq!(*doc.views.current().rows, [2, 1, 0]);
        for c in &mut spec.columns {
            c.sort = None;
        }
        doc.prepare_view(&spec)
            .and_then(|operation| operation.run())
            .unwrap();
        assert_eq!(*doc.views.current().rows, [0, 1, 2]);
    }

    #[test]
    fn hidden_columns_have_no_filter_and_invalid_specs_do_not_publish() {
        let doc = session();
        let mut spec =
            QuerySpec::all(3).with_filter(0, Filter::exact(vec!["1".into()], false, false));
        assert_eq!(
            doc.prepare_view(&spec)
                .and_then(|operation| operation.run())
                .unwrap()
                .row_count,
            2
        );
        spec.columns.retain(|c| c.column != 0);
        assert_eq!(
            doc.prepare_view(&spec)
                .and_then(|operation| operation.run())
                .unwrap()
                .row_count,
            3
        );
        let before = doc.current_view();
        let duplicate = QuerySpec {
            columns: vec![spec.columns[0].clone(), spec.columns[0].clone()],
        };
        let priorities = QuerySpec::all(3)
            .with_sort(0, false, 2)
            .with_sort(1, true, 2);
        let invalid = QuerySpec {
            columns: vec![ColSpec {
                column: 3,
                sort: None,
                filter: None,
            }],
        };
        for bad in [duplicate, priorities, invalid] {
            assert!(matches!(
                doc.prepare_view(&bad).and_then(|operation| operation.run()),
                Err(Error::InvalidArgument(_))
            ));
            assert!(matches!(
                doc.filter_choices(0, Some(&bad), &Cancellation::default()),
                Err(Error::InvalidArgument(_))
            ));
            assert_eq!(doc.current_view(), before);
        }
        assert_eq!(
            doc.prepare_view(&QuerySpec::new())
                .and_then(|operation| operation.run())
                .unwrap()
                .row_count,
            3
        );
        assert!(doc.views.current().spec.columns.is_empty());
    }
}
