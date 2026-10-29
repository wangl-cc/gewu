//! Which rows a facet (distinct values, histogram, summary) counts.

use std::sync::Arc;

use crate::ViewState;

pub(crate) enum RowScope<'a> {
    /// Every row of the file.
    All,
    /// Exactly these source rows, e.g. a view's.
    Rows(Arc<Vec<u32>>),
    /// Rows that pass `state`'s filters and search, ignoring the filters on
    /// column `except` (like Excel: a column's own filter does not hide
    /// that column's values).
    Facet { state: &'a ViewState, except: u32 },
}

impl<'a> RowScope<'a> {
    /// `Facet` when there is a state, otherwise `All`.
    pub fn facet(state: Option<&'a ViewState>, column: u32) -> Self {
        match state {
            Some(state) => Self::Facet {
                state,
                except: column,
            },
            None => Self::All,
        }
    }
}

/// The rows a [`RowScope`] resolved to.
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

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let (all, rows) = match self {
            Self::All(n) => (0..*n, &[][..]),
            Self::Rows(rows) => (0..0, rows.as_slice()),
        };
        all.chain(rows.iter().map(|&r| r as usize))
    }
}
