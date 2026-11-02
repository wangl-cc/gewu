//! Source rows selected for filter candidates, charts and analysis.

use std::sync::Arc;

use crate::QuerySpec;

pub(crate) enum RowScope<'a> {
    /// Every row of the file.
    All,
    /// Rows that pass `spec`'s filters, ignoring the filters on column
    /// `except` (like Excel: a column's own filter does not hide its values).
    Facet { spec: &'a QuerySpec, except: u32 },
}

impl<'a> RowScope<'a> {
    /// A facet with no other-column filters is the whole source; avoid
    /// allocating a source-row list just because a UI supplied a spec.
    pub fn facet(spec: Option<&'a QuerySpec>, column: u32) -> Self {
        match spec {
            Some(spec) => Self::Facet {
                spec,
                except: column,
            },
            None => Self::All,
        }
    }
}

/// The rows a [`RowScope`] resolved to.
#[derive(Debug)]
pub(crate) enum Selection {
    All(usize),
    Rows(Arc<Vec<u32>>),
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
            Self::Rows(rows) => (0..0, rows.as_slice()),
        };
        all.chain(rows.iter().map(|&r| r as usize))
    }
}
