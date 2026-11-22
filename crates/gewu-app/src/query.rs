//! Coalesce edits against the displayed table while one query is executing.
use std::sync::Arc;

use gewu_core::{Error, Result};

use crate::{AppliedView, TableQuery};

enum Phase {
    Idle,
    Running(Arc<TableQuery>),
    Failed(Error),
}

/// A document's displayed snapshot and latest requested sort/filter settings.
pub struct QuerySession {
    displayed: AppliedView,
    requested: Arc<TableQuery>,
    phase: Phase,
}

/// Immutable work that the caller can run away from the UI thread.
pub struct QueryJob {
    base: AppliedView,
    requested: Arc<TableQuery>,
}

/// The result and identity of a single query job.
pub struct QueryResult {
    requested: Arc<TableQuery>,
    result: Result<AppliedView>,
}

/// The change to the displayed table after delivering a completed query.
pub enum QueryCompletion {
    Unchanged,
    Installed,
    Run(QueryJob),
}

impl QueryJob {
    pub fn run(self) -> QueryResult {
        QueryResult {
            result: self.base.applying(self.requested.clone()),
            requested: self.requested,
        }
    }
}

impl QuerySession {
    pub fn new(displayed: AppliedView) -> Self {
        Self {
            requested: displayed.query().clone(),
            displayed,
            phase: Phase::Idle,
        }
    }

    pub fn displayed(&self) -> &AppliedView {
        &self.displayed
    }

    pub fn requested(&self) -> &TableQuery {
        &self.requested
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running(_))
    }

    pub fn error(&self) -> Option<&Error> {
        match &self.phase {
            Phase::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// Apply an edit to the latest request. An active job always finishes before a new one starts.
    pub fn edit(&mut self, edit: impl FnOnce(&mut TableQuery)) -> Option<QueryJob> {
        edit(Arc::make_mut(&mut self.requested));
        if self.is_running() {
            return None;
        }
        self.start()
    }

    fn start(&mut self) -> Option<QueryJob> {
        if &self.requested == self.displayed.query() {
            self.phase = Phase::Idle;
            return None;
        }
        self.phase = Phase::Running(self.requested.clone());
        Some(QueryJob {
            base: self.displayed.clone(),
            requested: self.requested.clone(),
        })
    }

    /// Publish only a current result. A failed current edit restores the displayed query as
    /// the starting point for the next edit. Obsolete failures and successes are discarded.
    pub fn finish(&mut self, completed: QueryResult) -> QueryCompletion {
        if !matches!(&self.phase, Phase::Running(active) if Arc::ptr_eq(active, &completed.requested))
        {
            return QueryCompletion::Unchanged;
        }
        if completed.requested != self.requested {
            return self
                .start()
                .map_or(QueryCompletion::Unchanged, QueryCompletion::Run);
        }
        match completed.result {
            Ok(view) => {
                self.requested = view.query().clone();
                self.displayed = view;
                self.phase = Phase::Idle;
                QueryCompletion::Installed
            }
            Err(error) => {
                self.requested = self.displayed.query().clone();
                self.phase = Phase::Failed(error);
                QueryCompletion::Unchanged
            }
        }
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
