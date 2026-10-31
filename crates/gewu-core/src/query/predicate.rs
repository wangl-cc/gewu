//! Prepared, column-bound predicates for high-throughput query execution.
//!
//! Predicates operate on 8K chunks (`CHUNK = 8 * 1024`), ensuring working data and masks
//! fit entirely inside the CPU's L1 data cache (<= 80 KB). Evaluators receive an optional incoming
//! mask (`incoming: Option<&[bool]>`), allowing:
//! - **Branchless SIMD**: Primitive comparisons compute unconditionally and bitwise-AND with the
//!   mask.
//! - **Branching Short-circuit**: Expensive string/complex operations skip rejected rows.
//! - **Chunk-level Circuit Breaking**: Fully-rejected chunks are bypassed entirely.

use std::ops::Range;

use super::{engine::CancelToken, selection::RowSelection};
use crate::Result;

pub(crate) const CHUNK: usize = 8 * 1024;

/// Chunk-level vectorized evaluator: populates the output boolean slice for a selection of rows,
/// optionally taking an incoming mask to support short-circuiting or branchless bitwise AND.
pub(crate) type ChunkEvaluator<'a> = Box<
    dyn for<'b> Fn(RowSelection<'b>, Option<&[bool]>, &mut [bool], &CancelToken) -> Result<()>
        + Send
        + Sync
        + 'a,
>;

pub(crate) struct Predicate<'a>(ChunkEvaluator<'a>);

impl<'a> Predicate<'a> {
    /// Construct a chunk-level vectorized predicate directly.
    pub(crate) fn new(evaluator: ChunkEvaluator<'a>) -> Self {
        Self(evaluator)
    }

    /// Construct a predicate that matches nothing (all false).
    pub(crate) fn empty() -> Self {
        Self::new(Box::new(|_rows, _incoming, output, _cancel| {
            output.fill(false);
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
            return Self::empty();
        }
        if predicates.len() == 1 {
            return predicates.into_iter().next().unwrap();
        }
        Self::new(Box::new(move |rows, incoming, output, cancel| {
            let mut scratch = vec![false; output.len()];
            for (i, predicate) in predicates.iter().enumerate() {
                cancel.check()?;
                if i == 0 {
                    // First alternative directly populates the chunk output with its matches.
                    predicate.evaluate_chunk(rows.clone(), incoming, output, cancel)?;
                } else {
                    // Fast circuit-break: if all active rows are already true, skip remaining
                    // predicates.
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

    /// Overwrite `matched` with results for the given rows (Range or borrowed slice of row
    /// indices).
    pub(crate) fn evaluate<'r>(
        &self,
        source_rows: impl Into<RowSelection<'r>>,
        matched: &mut [bool],
        cancel: &CancelToken,
    ) -> Result<()> {
        let source_rows = source_rows.into();
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
        for (chunk_idx, (out, scratch_chunk)) in matched
            .chunks_mut(CHUNK)
            .zip(scratch.chunks_mut(CHUNK))
            .enumerate()
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
