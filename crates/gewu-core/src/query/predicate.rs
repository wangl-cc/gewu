//! Prepared, column-bound predicates for high-throughput query execution.
//!
//! Predicates operate on 64K chunks (`CHUNK = 64 * 1024`), ensuring working data
//! fits entirely inside the CPU's L1 cache. Evaluators receive an optional incoming
//! mask (`incoming: Option<&[bool]>`), allowing:
//! - **Branchless SIMD**: Primitive comparisons compute unconditionally and bitwise-AND with the mask.
//! - **Branching Short-circuit**: Expensive string/complex operations skip rejected rows.
//! - **Chunk-level Circuit Breaking**: Fully-rejected chunks are bypassed entirely.

use std::ops::Range;

use super::engine::CancelToken;
use crate::Result;

pub(crate) const CHUNK: usize = 64 * 1024;

/// Chunk-level vectorized evaluator: populates the output boolean slice for a selection of rows,
/// optionally taking an incoming mask to support short-circuiting or branchless bitwise AND.
pub(crate) type ChunkEvaluator<'a> = Box<
    dyn for<'b> Fn(RowSelection<'b>, Option<&[bool]>, &mut [bool], &CancelToken) -> Result<()>
        + Send
        + Sync
        + 'a,
>;

/// Single-row scalar matcher closure: takes a row index and returns whether it matches.
pub(crate) type ScalarFilter<'a> = Box<dyn Fn(usize) -> bool + Send + Sync + 'a>;

/// Block-level evaluator: populates the output boolean slice for a selection of rows.
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

    pub(crate) fn chunk(&self, offset: usize, len: usize) -> Self {
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

pub(crate) struct Predicate<'a>(ChunkEvaluator<'a>);

impl<'a> Predicate<'a> {
    /// Construct a chunk-level vectorized predicate directly.
    pub(crate) fn chunk(evaluator: ChunkEvaluator<'a>) -> Self {
        Self(evaluator)
    }

    /// Construct a predicate from a scalar row matcher (adapted to chunk-level execution).
    pub(crate) fn scalar(is_match: ScalarFilter<'a>) -> Self {
        Self::chunk(Box::new(move |rows, incoming, output, _cancel| {
            match incoming {
                Some(mask) => {
                    for (i, &m) in mask.iter().enumerate() {
                        output[i] = m && is_match(rows.get(i));
                    }
                }
                None => {
                    for (i, hit) in output.iter_mut().enumerate() {
                        *hit = is_match(rows.get(i));
                    }
                }
            }
            Ok(())
        }))
    }

    /// Construct a predicate from a batch evaluator (adapted to chunk-level execution).
    pub(crate) fn batch(evaluator: BatchEvaluator<'a>) -> Self {
        Self::chunk(Box::new(move |rows, incoming, output, cancel| {
            evaluator(rows, output, cancel)?;
            if let Some(mask) = incoming {
                for (hit, &m) in output.iter_mut().zip(mask) {
                    *hit &= m;
                }
            }
            Ok(())
        }))
    }

    /// Evaluate a single chunk of rows.
    pub(crate) fn evaluate_chunk(
        &self,
        rows: RowSelection<'_>,
        incoming: Option<&[bool]>,
        output: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        (self.0)(rows, incoming, output, cancel)
    }

    /// Combine prepared equality alternatives without scanning the source or
    /// materializing one whole-column mask per value.
    pub(crate) fn any_of(predicates: Vec<Self>) -> Self {
        if predicates.is_empty() {
            return Self::chunk(Box::new(|_rows, _incoming, output, _cancel| {
                output.fill(false);
                Ok(())
            }));
        }
        if predicates.len() == 1 {
            return predicates.into_iter().next().unwrap();
        }
        Self::chunk(Box::new(move |rows, incoming, output, cancel| {
            output.fill(false);
            let mut scratch = vec![false; output.len()];
            for (i, predicate) in predicates.iter().enumerate() {
                cancel.check()?;
                if i == 0 {
                    predicate.evaluate_chunk(rows.clone(), incoming, output, cancel)?;
                } else {
                    // Fast circuit-break: if all active rows are already true, skip remaining predicates.
                    if let Some(mask) = incoming {
                        let all_true = mask.iter().zip(output.iter()).all(|(&m, &hit)| !m || hit);
                        if all_true {
                            break;
                        }
                    } else if output.iter().all(|&hit| hit) {
                        break;
                    }
                    scratch.fill(false);
                    predicate.evaluate_chunk(rows.clone(), incoming, &mut scratch, cancel)?;
                    for (hit, &s) in output.iter_mut().zip(&scratch) {
                        *hit |= s;
                    }
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
        for (chunk_idx, out) in matched.chunks_mut(CHUNK).enumerate() {
            cancel.check()?;
            let rows = source_rows.chunk(chunk_idx * CHUNK, out.len());
            (self.0)(rows, None, out, cancel)?;
        }
        cancel.check()
    }

    /// AND with existing matches. Leverages incoming mask and skips fully-rejected chunks.
    pub(crate) fn intersect(
        &self,
        source_rows: Range<usize>,
        matched: &mut [bool],
        scratch: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        assert_eq!(source_rows.len(), matched.len());
        assert_eq!(matched.len(), scratch.len());
        cancel.check()?;
        for (chunk_idx, (out, scratch_chunk)) in
            matched.chunks_mut(CHUNK).zip(scratch.chunks_mut(CHUNK)).enumerate()
        {
            cancel.check()?;
            // Fast chunk-level short-circuit: if all rows in this chunk are already false, skip it!
            if !out.iter().any(|&hit| hit) {
                continue;
            }
            let offset = chunk_idx * CHUNK;
            let rows = RowSelection::Range(
                source_rows.start + offset..source_rows.start + offset + out.len(),
            );
            (self.0)(rows, Some(out), scratch_chunk, cancel)?;
            out.copy_from_slice(scratch_chunk);
        }
        Ok(())
    }
}
