//! Capture cell-level Find snapshots without changing the current view.

use std::collections::BTreeSet;

use super::View;
use crate::{Error, Find, Result};

impl View {
    /// Capture a view, query and ordered visible-column scope for cell Find.
    /// Preparation is cheap and does not scan rows. The returned session remains
    /// valid after later view changes and owns independent cancellation.
    pub fn find(&self, query: &str, columns: &[u32]) -> Result<Find> {
        let mut seen = BTreeSet::new();
        let columns = columns
            .iter()
            .map(|&column| {
                if !seen.insert(column) {
                    return Err(Error::InvalidArgument("duplicate find column".into()));
                }
                self.column_array(column)
                    .cloned()
                    .map(|array| (column, array))
            })
            .collect::<Result<Vec<_>>>()?;
        Find::new(columns, self.rows.clone(), query.to_owned())
    }
}
