//! Cooperative, terminal cancellation shared by an operation and its kernels.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{Error, Result};

#[derive(Clone, Default)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
    parents: Vec<Self>,
    #[cfg(test)]
    checks_left: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl Cancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn check(&self) -> Result<()> {
        #[cfg(test)]
        if let Some(left) = &self.checks_left {
            if left
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_sub(1))
                })
                .unwrap()
                <= 1
            {
                self.cancel();
            }
        }
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        for parent in &self.parents {
            parent.check()?;
        }
        Ok(())
    }

    pub(crate) fn check_at(&self, index: usize) -> Result<()> {
        if index.is_multiple_of(4096) {
            self.check()?;
        }
        Ok(())
    }

    /// Observe two scopes without allowing a child to cancel its parents.
    pub(crate) fn combined_with(&self, other: &Self) -> Self {
        Self {
            parents: vec![self.clone(), other.clone()],
            ..Self::default()
        }
    }

    /// Cancel at a deterministic checkpoint while exercising real kernels.
    #[cfg(test)]
    pub(crate) fn after_checks(checks: usize) -> Self {
        Self {
            checks_left: Some(Arc::new(std::sync::atomic::AtomicUsize::new(checks))),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_tokens_observe_every_constituent() {
        for cancelled in 0..3 {
            let first = Cancellation::default();
            let second = Cancellation::default();
            let third = Cancellation::default();
            let combined = first.combined_with(&second).combined_with(&third);
            assert!(combined.check().is_ok());
            [first, second, third][cancelled].cancel();
            assert!(matches!(combined.check(), Err(Error::Cancelled)));
        }
    }

    #[test]
    fn child_cancellation_is_terminal_and_does_not_cancel_parents() {
        let session = Cancellation::default();
        let page = Cancellation::default();
        let child = session.combined_with(&page);
        page.cancel();
        page.cancel();
        assert!(matches!(child.check(), Err(Error::Cancelled)));
        session.check().unwrap();
        session
            .combined_with(&Cancellation::default())
            .check()
            .unwrap();
        session.cancel();
        assert!(matches!(
            session.combined_with(&Cancellation::default()).check(),
            Err(Error::Cancelled)
        ));
    }
}
