//! Prepared, column-bound predicates for high-throughput query execution.
//!
//! Two execution tracks are supported:
//! - **Scalar**: Per-row scalar test (`Fn(usize) -> bool`). Highly efficient for primitive
//!   comparisons and dictionary codes; supports immediate AND short-circuiting.
//! - **Batch**: Block-level vectorized evaluation (`Fn(RowSelection, &mut [bool], ...)`). Allows
//!   reusing formatting scratch buffers across rows (e.g. text Contains or nested types).

use std::ops::Range;

use super::engine::CancelToken;
use crate::Result;

pub(crate) const CHUNK: usize = 64 * 1024;

/// Single-row scalar matcher closure: takes a row index and returns whether it matches.
pub(crate) type ScalarFilter<'a> = Box<dyn Fn(usize) -> bool + Send + Sync + 'a>;

/// Block-level vectorized evaluator: populates the output boolean slice for a selection of rows.
pub(crate) type BatchEvaluator<'a> = Box<
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
    /// Scalar per-row evaluation: native typed comparisons with AND short-circuiting.
    Scalar(ScalarFilter<'a>),
    /// Vectorized chunk evaluation: buffer reuse for text formatting or nested types.
    Batch(BatchEvaluator<'a>),
}

impl<'a> Predicate<'a> {
    pub(crate) fn scalar(filter: ScalarFilter<'a>) -> Self {
        Self::Scalar(filter)
    }

    pub(crate) fn batch(evaluator: BatchEvaluator<'a>) -> Self {
        Self::Batch(evaluator)
    }

    /// Combine prepared equality alternatives without scanning the source or
    /// materializing one whole-column mask per value.
    pub(crate) fn any_of(predicates: Vec<Self>) -> Self {
        let mut scalars = Vec::new();
        let mut batches = Vec::new();
        for predicate in predicates {
            match predicate {
                Self::Scalar(filter) => scalars.push(filter),
                Self::Batch(evaluator) => batches.push(Self::Batch(evaluator)),
            }
        }
        let scalar_pred = Self::scalar(Box::new(move |row| {
            scalars.iter().any(|is_match| is_match(row))
        }));
        if batches.is_empty() {
            return scalar_pred;
        }
        Self::batch(Box::new(move |selection, hits, cancel| {
            scalar_pred.evaluate_selected(selection.clone(), hits, cancel)?;
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
            Self::Scalar(is_match) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    let rows = source_rows.chunk(chunk * CHUNK, out.len());
                    for (index, hit) in (0..).zip(out) {
                        *hit = is_match(rows.get(index));
                    }
                }
            }
            Self::Batch(evaluator) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    out.fill(false);
                    evaluator(source_rows.chunk(chunk * CHUNK, out.len()), out, cancel)?;
                }
            }
        }
        cancel.check()
    }

    /// AND with existing matches. Native scalar checks short-circuit rejected rows.
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
            Self::Scalar(is_match) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    let start = source_rows.start + chunk * CHUNK;
                    for (row, hit) in (start..).zip(out) {
                        if *hit {
                            *hit = is_match(row);
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
