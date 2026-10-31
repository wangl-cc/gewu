//! Capture cell-level Find snapshots without changing the current view.

use std::collections::BTreeSet;

use super::Document;
use crate::{Error, Find, Result};

impl Document {
    /// Capture a view, query and ordered visible-column scope for cell Find.
    /// Preparation is cheap and does not scan rows. The returned session remains
    /// valid after later view changes and owns independent cancellation.
    pub fn prepare_find(&self, version: u64, query: &str, columns: &[u32]) -> Result<Find> {
        let view = self.views.snapshot(version)?;
        let mut seen = BTreeSet::new();
        let columns = columns
            .iter()
            .map(|&column| {
                if !seen.insert(column) {
                    return Err(Error::InvalidArgument("duplicate find column".into()));
                }
                self.column(column).cloned().map(|array| (column, array))
            })
            .collect::<Result<Vec<_>>>()?;
        Find::new(columns, view.rows.clone(), version, query.to_owned())
    }
}
