//! The current view, its version, and cancellation of view changes.
//!
//! A view is a list of source rows in display order. Every committed view gets
//! a new version; readers name the version they want, and a request for an old
//! one fails with [`Error::StaleVersion`] instead of showing the wrong rows.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};

use crate::{Error, Result, ViewInfo};

/// Tells a long-running operation that a newer one has replaced it.
#[derive(Clone)]
pub struct CancelToken {
    counter: Arc<AtomicU64>,
    mine: u64,
}

impl CancelToken {
    /// A token that is never cancelled, for work nobody replaces.
    pub(crate) fn never() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(0)),
            mine: 0,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.counter.load(Ordering::SeqCst) != self.mine
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub(crate) struct View {
    pub version: u64,
    /// Source row numbers in display order.
    pub rows: Arc<Vec<u32>>,
    /// The search that produced this view, for marking matched cells.
    pub search: Option<String>,
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
    current: Mutex<Arc<View>>,
    generation: Arc<AtomicU64>,
    next_version: AtomicU64,
}

impl ViewEngine {
    /// Start with every row in file order, as version 0.
    pub fn new(n_rows: usize) -> Self {
        Self {
            current: Mutex::new(Arc::new(View {
                version: 0,
                rows: Arc::new((0..n_rows as u32).collect()),
                search: None,
            })),
            generation: Arc::new(AtomicU64::new(0)),
            next_version: AtomicU64::new(1),
        }
    }

    /// The view is only ever replaced whole, so a panic elsewhere cannot leave
    /// it half-written; keep serving it rather than poisoning the document.
    fn lock(&self) -> MutexGuard<'_, Arc<View>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn current(&self) -> Arc<View> {
        self.lock().clone()
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
    pub fn begin(&self) -> CancelToken {
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        CancelToken {
            counter: self.generation.clone(),
            mine,
        }
    }

    /// Cancel any view change still running.
    pub fn cancel_pending(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Switch to `rows` unless `token` was cancelled. The check happens under
    /// the lock, so a cancelled change can never be committed.
    pub fn commit(
        &self,
        token: &CancelToken,
        rows: Vec<u32>,
        search: Option<String>,
    ) -> Result<ViewInfo> {
        let mut current = self.lock();
        token.check()?;
        let view = View {
            version: self.next_version.fetch_add(1, Ordering::SeqCst),
            rows: Arc::new(rows),
            search,
        };
        let info = view.info();
        *current = Arc::new(view);
        Ok(info)
    }
}
