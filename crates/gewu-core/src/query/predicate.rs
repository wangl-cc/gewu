//! Prepared, column-bound predicates. Preparation does not scan rows.
//!
//! Row predicates retain native typed comparisons and can short-circuit an AND.
//! Batch predicates handle formatted fallback or nested arrays.
//! Selections borrow either a contiguous range or ordered source-row indices;
//! output slots follow selection order and never require copying Arrow value payload.

use std::ops::Range;

use super::engine::CancelToken;
use crate::Result;

pub(crate) const CHUNK: usize = 64 * 1024;
pub(crate) type RowTest<'a> = Box<dyn Fn(usize) -> bool + Send + Sync + 'a>;
type BatchTest<'a> = Box<
    dyn for<'b> Fn(RowSelection<'b>, &mut [bool], &CancelToken) -> Result<()> + Send + Sync + 'a,
>;

#[derive(Clone)]
pub(crate) enum RowSelection<'a> {
    Range(Range<usize>),
    Indices(&'a [usize]),
}

impl<'a> RowSelection<'a> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Range(rows) => rows.len(),
            Self::Indices(rows) => rows.len(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> usize {
        match self {
            Self::Range(rows) => rows.start + index,
            Self::Indices(rows) => rows[index],
        }
    }

    pub(crate) fn iter(&self) -> RowSelectionIter<'_> {
        match self {
            Self::Range(rows) => RowSelectionIter::Range(rows.clone()),
            Self::Indices(rows) => RowSelectionIter::Indices(rows.iter()),
        }
    }

    fn chunk(&self, offset: usize, len: usize) -> Self {
        match self {
            Self::Range(rows) => Self::Range(rows.start + offset..rows.start + offset + len),
            Self::Indices(rows) => Self::Indices(&rows[offset..offset + len]),
        }
    }
}

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

pub(crate) enum Predicate<'a> {
    Rows(RowTest<'a>),
    Batch(BatchTest<'a>),
}

impl<'a> Predicate<'a> {
    pub(crate) fn rows(test: RowTest<'a>) -> Self {
        Self::Rows(test)
    }

    pub(crate) fn batch(test: BatchTest<'a>) -> Self {
        Self::Batch(test)
    }

    /// Combine prepared equality alternatives without scanning the source or
    /// materializing one whole-column mask per value.
    pub(crate) fn any_of(predicates: Vec<Self>) -> Self {
        let mut rows = Vec::new();
        let mut batches = Vec::new();
        for predicate in predicates {
            match predicate {
                Self::Rows(test) => rows.push(test),
                Self::Batch(test) => batches.push(Self::Batch(test)),
            }
        }
        let rows = Self::rows(Box::new(move |row| rows.iter().any(|test| test(row))));
        if batches.is_empty() {
            return rows;
        }
        Self::batch(Box::new(move |selection, hits, cancel| {
            rows.evaluate_selected(selection.clone(), hits, cancel)?;
            let mut scratch = vec![false; selection.len()];
            for predicate in &batches {
                predicate.evaluate_selected(selection.clone(), &mut scratch, cancel)?;
                for (hit, other) in hits.iter_mut().zip(&scratch) {
                    *hit |= *other;
                }
            }
            Ok(())
        }))
    }

    /// Overwrite the output with matches for this source range.
    pub(crate) fn evaluate(
        &self,
        source_rows: Range<usize>,
        matched: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        self.evaluate_selected(RowSelection::Range(source_rows), matched, cancel)
    }

    /// Overwrite the output with matches for an ordered borrowed selection.
    pub(crate) fn evaluate_selected(
        &self,
        source_rows: RowSelection<'_>,
        matched: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        assert_eq!(source_rows.len(), matched.len());
        cancel.check()?;
        match self {
            Self::Rows(test) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    let rows = source_rows.chunk(chunk * CHUNK, out.len());
                    for (index, hit) in (0..).zip(out) {
                        *hit = test(rows.get(index));
                    }
                }
            }
            Self::Batch(test) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    out.fill(false);
                    test(source_rows.chunk(chunk * CHUNK, out.len()), out, cancel)?;
                }
            }
        }
        cancel.check()
    }

    /// AND with existing matches. Native row tests skip rejected rows.
    pub(crate) fn intersect(
        &self,
        source_rows: Range<usize>,
        matched: &mut [bool],
        scratch: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        assert_eq!(source_rows.len(), matched.len());
        cancel.check()?;
        match self {
            Self::Rows(test) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    let start = source_rows.start + chunk * CHUNK;
                    for (row, hit) in (start..).zip(out) {
                        if *hit {
                            *hit = test(row);
                        }
                    }
                }
                Ok(())
            }
            _ => {
                self.evaluate(source_rows, scratch, cancel)?;
                for (hit, other) in matched.iter_mut().zip(scratch) {
                    *hit &= *other;
                }
                Ok(())
            }
        }
    }
}
