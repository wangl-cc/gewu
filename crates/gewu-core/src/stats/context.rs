use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Error, Result};

#[derive(Default)]
pub(crate) struct AnalysisContext {
    cancelled: AtomicBool,
}

impl AnalysisContext {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn check_at(&self, index: usize) -> Result<()> {
        if index.is_multiple_of(4096) {
            self.check()?;
        }
        Ok(())
    }
}
