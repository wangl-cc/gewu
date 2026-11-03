//! Source rows selected for filter candidates, charts and analysis.

use std::sync::Arc;

/// Source rows for order-independent scans.
#[derive(Debug)]
pub(crate) enum Selection {
    All(usize),
    Rows(Arc<[u32]>),
}

impl Selection {
    pub fn len(&self) -> usize {
        match self {
            Self::All(n) => *n,
            Self::Rows(rows) => rows.len(),
        }
    }

    /// The selected source rows in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let (all, rows) = match self {
            Self::All(n) => (0..*n, &[][..]),
            Self::Rows(rows) => (0..0, rows.as_ref()),
        };
        all.chain(rows.iter().map(|&r| r as usize))
    }
}
