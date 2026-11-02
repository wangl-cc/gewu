//! The current view, its version, and cancellation of view changes.
//!
//! A view is a list of source rows in display order. Every committed view gets
//! a new version; readers name the version they want, and a request for an old
//! one fails with [`Error::StaleVersion`] instead of showing the wrong rows.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Cancellation, Error, QuerySpec, Result, ViewInfo};

pub(crate) struct View {
    pub spec: QuerySpec,
    pub version: u64,
    /// Source row numbers in display order.
    pub rows: Arc<Vec<u32>>,
}

impl View {
    pub fn info(&self) -> ViewInfo {
        ViewInfo {
            version: self.version,
            row_count: self.rows.len() as u64,
        }
    }
}

pub(crate) struct ViewEngine {
    state: Mutex<State>,
}

struct State {
    current: Arc<View>,
    pending: Option<Cancellation>,
}

impl ViewEngine {
    /// Start with every row in file order, as version 0.
    pub fn new(n_rows: usize, n_columns: usize) -> Self {
        Self {
            state: Mutex::new(State {
                current: Arc::new(View {
                    spec: QuerySpec::all(n_columns),
                    version: 0,
                    rows: Arc::new((0..n_rows as u32).collect()),
                }),
                pending: None,
            }),
        }
    }

    /// The view is only ever replaced whole, so a panic elsewhere cannot leave
    /// it half-written; keep serving it rather than poisoning the document.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn current(&self) -> Arc<View> {
        self.lock().current.clone()
    }

    /// The current view, if it is still `version`.
    pub fn snapshot(&self, version: u64) -> Result<Arc<View>> {
        let view = self.current();
        if view.version == version {
            Ok(view)
        } else {
            Err(Error::StaleVersion {
                requested: version,
                current: view.version,
            })
        }
    }

    /// Start a view change. Starting another one cancels this one.
    pub fn begin(&self) -> Cancellation {
        let mut state = self.lock();
        if let Some(previous) = state.pending.take() {
            previous.cancel();
        }
        let token = Cancellation::default();
        state.pending = Some(token.clone());
        token
    }

    /// Switch to `rows` unless `token` was cancelled. The final check under
    /// the view lock is the commit linearization point: a concurrent cancel
    /// after that check does not roll back this already accepted publication.
    pub fn commit(
        &self,
        token: &Cancellation,
        rows: Arc<Vec<u32>>,
        spec: QuerySpec,
    ) -> Result<ViewInfo> {
        let mut state = self.lock();
        token.check()?;
        let view = View {
            version: state.current.version + 1,
            rows,
            spec,
        };
        let info = view.info();
        state.current = Arc::new(view);
        state.pending = None;
        Ok(info)
    }
}
