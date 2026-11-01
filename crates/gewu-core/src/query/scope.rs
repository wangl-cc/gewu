//! Which rows a facet (distinct values, histogram, summary) counts.

use std::sync::Arc;

use crate::ViewState;

pub(crate) enum RowScope<'a> {
    /// Every row of the file.
    All,
    /// Exactly these source rows, e.g. a view's, each at most once. Their
    /// order does not matter to anything that counts them.
    Rows(Arc<Vec<u32>>),
    /// Rows that pass `state`'s filters, ignoring the filters on column
    /// `except` (like Excel: a column's own filter does not hide its values).
    Facet { state: &'a ViewState, except: u32 },
}

impl<'a> RowScope<'a> {
    /// A facet with no other-column filters is the whole source; avoid
    /// allocating a source-row list just because a UI supplied a state.
    pub fn facet(state: Option<&'a ViewState>, column: u32) -> Self {
        match state {
            Some(state)
                if state
                    .filters
                    .iter()
                    .any(|(&col, filters)| col != column && !filters.is_empty()) =>
            {
                Self::Facet {
                    state,
                    except: column,
                }
            }
            _ => Self::All,
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

    /// The rows in order. In a hot loop prefer [`for_rows!`], which
    /// compiles the loop separately for each case and runs faster.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let (all, rows) = match self {
            Self::All(n) => (0..*n, &[][..]),
            Self::Rows(rows) => (0..0, rows.as_slice()),
        };
        all.chain(rows.iter().map(|&r| r as usize))
    }
}

/// Run `$body` with `$rows` bound to an iterator over a [`Selection`]'s rows,
/// with a separate, fully optimised copy of `$body` for each case.
macro_rules! for_rows {
    ($selection:expr, | $rows:ident | $body:expr) => {
        match $selection {
            $crate::scope::Selection::All(n) => {
                let $rows = 0..*n;
                $body
            }
            $crate::scope::Selection::Rows(rows) => {
                let $rows = rows.iter().map(|&r| r as usize);
                $body
            }
        }
    };
}
pub(crate) use for_rows;
