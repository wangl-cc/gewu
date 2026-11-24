//! Immutable application snapshots shared by queries, analyses and table windows.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;

use gewu_core::{ColumnFilter, ColumnSort, Dataset, FilteredView, OrderedView, Result};

mod analysis;
pub mod find;
mod query;
pub use analysis::Analyses;
pub use query::{QueryCompletion, QueryJob, QueryResult, QuerySession};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TableQuery {
    pub filters: Vec<ColumnFilter>,
    pub sorts: Vec<ColumnSort>,
}

impl TableQuery {
    /// Filters form a conjunction. Sort keys keep their user-specified priority.
    fn order_filters(query: &mut Arc<Self>) {
        if !query.filters.is_sorted_by_key(|filter| filter.column) {
            Arc::make_mut(query)
                .filters
                .sort_by_key(|filter| filter.column);
        }
    }
}

/// One displayed result. Cloning preserves the identity of its dataset, rows and analyses.
#[derive(Clone)]
pub struct AppliedView {
    dataset: Arc<Dataset>,
    query: Arc<TableQuery>,
    analyses: Arc<Analyses>,
    view: Arc<OrderedView>,
}

impl AppliedView {
    pub fn new(dataset: Arc<Dataset>) -> Self {
        let filtered = dataset.unfiltered();
        let view = filtered.unsorted();
        Self {
            dataset,
            query: Arc::default(),
            analyses: Arc::new(Analyses::new(filtered)),
            view,
        }
    }

    /// Apply to the full dataset. An unchanged query retains the ordered view; changing
    /// only the sort retains the filter result and all its cached analyses.
    pub fn applying(&self, mut query: Arc<TableQuery>) -> Result<Self> {
        TableQuery::order_filters(&mut query);
        self.apply_ordered_query(query)
    }

    fn apply_ordered_query(&self, query: Arc<TableQuery>) -> Result<Self> {
        if query == self.query {
            return Ok(self.clone());
        }
        let analyses = if query.filters == self.query.filters {
            self.analyses.clone()
        } else {
            Arc::new(Analyses::new(self.dataset.filter(&query.filters)?))
        };
        let view = analyses.rows.sort(&query.sorts)?;
        Ok(Self {
            dataset: self.dataset.clone(),
            query,
            analyses,
            view,
        })
    }

    pub fn dataset(&self) -> &Arc<Dataset> {
        &self.dataset
    }

    pub fn query(&self) -> &Arc<TableQuery> {
        &self.query
    }

    pub fn filtered(&self) -> &Arc<FilteredView> {
        &self.analyses.rows
    }

    pub fn view(&self) -> &Arc<OrderedView> {
        &self.view
    }

    pub fn analyses(&self) -> &Analyses {
        &self.analyses
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
