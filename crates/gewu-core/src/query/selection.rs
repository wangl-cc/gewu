//! Zero-allocation row addressing and selection vectors.
//!
//! Evaluators and query execution kernels receive a `RowSelection`, allowing:
//! - **Contiguous Range (`0..N`)**: Zero memory overhead, direct slice access, and automatic SIMD.
//! - **Gather / Indirect Indices (`&[usize]`)**: Reuses borrowed index buffers from sorting,
//!   dictionary lookups, or search without copying or materializing row vectors.

use std::ops::Range;

/// Borrowed, zero-allocation row selection abstraction representing either a contiguous range
/// or a sequence of discrete row indices.
#[derive(Clone)]
pub(crate) enum RowSelection<'a> {
    Range(Range<usize>),
    Indices(&'a [usize]),
}

impl<'a> RowSelection<'a> {
    /// Number of rows in this selection.
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Range(rows) => rows.len(),
            Self::Indices(rows) => rows.len(),
        }
    }

    /// Iterate over row indices in this selection.
    pub(crate) fn iter(&self) -> RowSelectionIter<'_> {
        match self {
            Self::Range(rows) => RowSelectionIter::Range(rows.clone()),
            Self::Indices(rows) => RowSelectionIter::Indices(rows.iter()),
        }
    }

    /// Slice a sub-chunk of this row selection by offset and length.
    pub(crate) fn chunk(&self, offset: usize, len: usize) -> Self {
        match self {
            Self::Range(rows) => Self::Range(rows.start + offset..rows.start + offset + len),
            Self::Indices(rows) => Self::Indices(&rows[offset..offset + len]),
        }
    }
}

impl<'a> From<Range<usize>> for RowSelection<'a> {
    fn from(range: Range<usize>) -> Self {
        Self::Range(range)
    }
}

impl<'a> From<&'a [usize]> for RowSelection<'a> {
    fn from(indices: &'a [usize]) -> Self {
        Self::Indices(indices)
    }
}

impl<'a> From<&'a Vec<usize>> for RowSelection<'a> {
    fn from(indices: &'a Vec<usize>) -> Self {
        Self::Indices(indices.as_slice())
    }
}

/// Iterator over row indices in a `RowSelection`.
pub(crate) enum RowSelectionIter<'a> {
    Range(Range<usize>),
    Indices(std::slice::Iter<'a, usize>),
}

impl Iterator for RowSelectionIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Range(rows) => rows.next(),
            Self::Indices(rows) => rows.next().copied(),
        }
    }
}
