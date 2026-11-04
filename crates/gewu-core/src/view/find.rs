//! Capture cell-level Find snapshots without changing the current view.

use std::collections::BTreeSet;

use super::View;
use crate::{Find, Result};

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
                    return Err(crate::ColumnError::Duplicate {
                        operation: crate::ColumnOperation::Find,
                        column,
                    });
                }
                self.column_array(column)
                    .cloned()
                    .map(|array| (column, array))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Find::new(columns, self.rows.clone(), query.to_owned())?)
    }
}
