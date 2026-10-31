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
pub(crate) struct CancelToken {
    counter: Arc<AtomicU64>,
    mine: u64,
    additional: Vec<Arc<CancelToken>>,
}

impl CancelToken {
    /// A token that is never cancelled, for work nobody replaces.
    pub(crate) fn never() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(0)),
            mine: 0,
            additional: Vec::new(),
        }
    }

    /// A token and the independent counter used to cancel it.
    pub(crate) fn independent() -> (Self, Arc<AtomicU64>) {
        let counter = Arc::new(AtomicU64::new(0));
        (
            Self {
                counter: counter.clone(),
                mine: 0,
                additional: Vec::new(),
            },
            counter,
        )
    }

    /// Observe this scope and another one; cancelling either cancels the result.
    pub(crate) fn combined_with(&self, other: &Self) -> Self {
        let mut additional = self.additional.clone();
        additional.push(Arc::new(other.clone()));
        Self {
            counter: self.counter.clone(),
            mine: self.mine,
            additional,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.counter.load(Ordering::SeqCst) != self.mine
            || self.additional.iter().any(|parent| parent.is_cancelled())
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
            additional: Vec::new(),
        }
    }

    /// Cancel any view change still running.
    pub fn cancel_pending(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Switch to `rows` unless `token` was cancelled. The final check under
    /// the view lock is the commit linearization point: a concurrent cancel
    /// after that check does not roll back this already accepted publication.
    pub fn commit(&self, token: &CancelToken, rows: Vec<u32>) -> Result<ViewInfo> {
        let mut current = self.lock();
        token.check()?;
        let view = View {
            version: self.next_version.fetch_add(1, Ordering::SeqCst),
            rows: Arc::new(rows),
        };
        let info = view.info();
        *current = Arc::new(view);
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_tokens_observe_every_constituent() {
        for cancelled in 0..3 {
            let (first, first_cancel) = CancelToken::independent();
            let (second, second_cancel) = CancelToken::independent();
            let (third, third_cancel) = CancelToken::independent();
            let combined = first.combined_with(&second).combined_with(&third);
            assert!(combined.check().is_ok());
            [first_cancel, second_cancel, third_cancel][cancelled].fetch_add(1, Ordering::SeqCst);
            assert!(matches!(combined.check(), Err(Error::Cancelled)));
        }
    }
}
