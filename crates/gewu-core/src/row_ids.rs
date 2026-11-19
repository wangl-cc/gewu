//! Zero-allocation row ID representations for unfiltered, filtered, and sorted views.

use std::{ops::Range, sync::Arc};

/// Owned row selection shared across views, Find sessions, and exports.
#[derive(Clone)]
pub(crate) enum RowIds {
    /// All rows `0..n` in source order, with no backing allocation.
    All(usize),
    /// Explicit list of source row IDs in display order.
    Selected(Arc<[u32]>),
}

impl RowIds {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::All(n) => *n,
            Self::Selected(rows) => rows.len(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn as_slice(&self) -> RowSlice<'_> {
        match self {
            Self::All(n) => RowSlice::All(*n),
            Self::Selected(rows) => RowSlice::Selected(rows),
        }
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::All(a), Self::All(b)) => a == b,
            (Self::Selected(a), Self::Selected(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Borrowed view over [`RowIds`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowSlice<'a> {
    All(usize),
    Selected(&'a [u32]),
}

impl<'a> RowSlice<'a> {
    pub(crate) fn len(self) -> usize {
        match self {
            Self::All(n) => n,
            Self::Selected(rows) => rows.len(),
        }
    }

    pub(crate) fn iter(self) -> RowIter<'a> {
        match self {
            Self::All(n) => RowIter::Range(0..n),
            Self::Selected(rows) => RowIter::Slice(rows.iter()),
        }
    }

    pub(crate) fn iter_u32(self) -> RowU32Iter<'a> {
        match self {
            Self::All(n) => RowU32Iter::Range(0..n as u32),
            Self::Selected(rows) => RowU32Iter::Slice(rows.iter()),
        }
    }

    pub(crate) fn window(self, start: usize, end: usize) -> RowWindow<'a> {
        let end = end.min(self.len());
        let start = start.min(end);
        match self {
            Self::All(_) => RowWindow::Range { start, end },
            Self::Selected(rows) => RowWindow::Slice(&rows[start..end]),
        }
    }

    #[cfg(any(feature = "parquet", feature = "csv", feature = "xlsx"))]
    pub(crate) fn chunks(self, chunk_size: usize) -> impl Iterator<Item = RowWindow<'a>> {
        let len = self.len();
        (0..len)
            .step_by(chunk_size)
            .map(move |start| self.window(start, (start + chunk_size).min(len)))
    }
}

/// Bounded sub-window of a [`RowSlice`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowWindow<'a> {
    Range { start: usize, end: usize },
    Slice(&'a [u32]),
}

impl<'a> RowWindow<'a> {
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Range { start, end } => end - start,
            Self::Slice(rows) => rows.len(),
        }
    }

    pub(crate) fn iter(self) -> RowIter<'a> {
        match self {
            Self::Range { start, end } => RowIter::Range(start..end),
            Self::Slice(rows) => RowIter::Slice(rows.iter()),
        }
    }

    #[cfg_attr(
        not(any(feature = "parquet", feature = "csv")),
        allow(
            dead_code,
            reason = "Only batch writers use iter_u32 when codecs are enabled"
        )
    )]
    pub(crate) fn iter_u32(self) -> RowU32Iter<'a> {
        match self {
            Self::Range { start, end } => RowU32Iter::Range(start as u32..end as u32),
            Self::Slice(rows) => RowU32Iter::Slice(rows.iter()),
        }
    }

    pub(crate) fn contiguous_range(self) -> Option<Range<usize>> {
        match self {
            Self::Range { start, end } => Some(start..end),
            Self::Slice([]) => Some(0..0),
            Self::Slice(rows) => {
                if rows
                    .windows(2)
                    .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
                {
                    let start = rows[0] as usize;
                    Some(start..start + rows.len())
                } else {
                    None
                }
            }
        }
    }
}

pub(crate) enum RowIter<'a> {
    Range(Range<usize>),
    Slice(std::slice::Iter<'a, u32>),
}

impl Iterator for RowIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Range(range) => range.next(),
            Self::Slice(iter) => iter.next().map(|&r| r as usize),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Range(range) => range.size_hint(),
            Self::Slice(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for RowIter<'_> {}

pub(crate) enum RowU32Iter<'a> {
    Range(Range<u32>),
    Slice(std::slice::Iter<'a, u32>),
}

impl Iterator for RowU32Iter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Range(range) => range.next(),
            Self::Slice(iter) => iter.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Range(range) => range.size_hint(),
            Self::Slice(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for RowU32Iter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_ids_and_windows_behave_consistently() {
        let all = RowIds::All(5);
        assert_eq!(all.len(), 5);
        assert!(!all.is_empty());
        assert!(all.ptr_eq(&RowIds::All(5)));
        assert!(!all.ptr_eq(&RowIds::All(4)));

        let slice = all.as_slice();
        assert_eq!(slice.len(), 5);
        assert_eq!(RowSlice::All(0).len(), 0);
        assert_eq!(slice.iter().collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
        assert_eq!(slice.iter_u32().collect::<Vec<_>>(), [0, 1, 2, 3, 4]);

        let win = slice.window(1, 4);
        assert_eq!(win.len(), 3);
        assert_eq!(win.contiguous_range(), Some(1..4));
        assert_eq!(win.iter().collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(win.iter_u32().collect::<Vec<_>>(), [1, 2, 3]);

        let selected = RowIds::Selected(Arc::from([10_u32, 11, 12, 20]));
        let cloned = selected.clone();
        assert!(selected.ptr_eq(&cloned));
        assert!(!selected.ptr_eq(&all));
        assert!(!selected.ptr_eq(&RowIds::Selected(Arc::from([10_u32, 11, 12, 20]))));

        let sel_slice = selected.as_slice();
        assert_eq!(sel_slice.window(0, 3).contiguous_range(), Some(10..13));
        assert_eq!(sel_slice.window(0, 4).contiguous_range(), None);
        assert_eq!(sel_slice.window(4, 4).contiguous_range(), Some(0..0));
        assert_eq!(sel_slice.window(4, 4).len(), 0);
    }
}
