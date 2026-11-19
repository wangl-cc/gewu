//! Compiled, column-bound predicates for high-throughput query execution.
//!
//! A predicate has two evaluators over the same semantics:
//! - **Selection bitmap** ([`Predicate::select`]): filters evaluate each condition over a
//!   contiguous source range into a packed [`BooleanBuffer`]. Hot predicates compare native values
//!   64 rows per word and apply nulls with bitwise operations on the validity bitmap; the others
//!   are packed from their chunk evaluator.
//! - **Chunk evaluator** ([`Predicate::evaluate`]): Find evaluates display-ordered row IDs in 8K
//!   chunks (`CHUNK = 8 * 1024`), so working data and masks stay in the L1 data cache.

use std::ops::Range;

use arrow::{
    array::BooleanBufferBuilder,
    buffer::{BooleanBuffer, NullBuffer},
};

use super::selection::RowSelection;

pub(crate) const CHUNK: usize = 8 * 1024;

/// Chunk-level vectorized evaluator: overwrites the output boolean slice for a selection of rows.
pub(crate) type ChunkEvaluator<'a> =
    Box<dyn for<'b> Fn(RowSelection<'b>, &mut [bool]) + Send + Sync + 'a>;

/// Whole-range evaluator: returns one bit per source row in the range.
pub(crate) type BitmapEvaluator<'a> = Box<dyn Fn(Range<usize>) -> BooleanBuffer + Send + Sync + 'a>;

pub(crate) struct Predicate<'a> {
    chunk: ChunkEvaluator<'a>,
    bitmap: Option<BitmapEvaluator<'a>>,
}

impl<'a> Predicate<'a> {
    /// Construct a chunk-level vectorized predicate directly.
    pub(crate) fn new(evaluator: ChunkEvaluator<'a>) -> Self {
        Self {
            chunk: evaluator,
            bitmap: None,
        }
    }

    /// Attach a native bitmap evaluator. It must agree with the chunk evaluator row for row.
    pub(crate) fn with_bitmap(mut self, bitmap: BitmapEvaluator<'a>) -> Self {
        self.bitmap = Some(bitmap);
        self
    }

    /// Construct a predicate that matches nothing (all false).
    pub(crate) fn empty() -> Self {
        Self::constant(false)
    }

    /// Construct a predicate that matches every row (`true`) or none (`false`).
    pub(crate) fn constant(hit: bool) -> Self {
        Self::new(Box::new(move |_rows, output| output.fill(hit))).with_bitmap(Box::new(
            move |rows| {
                if hit {
                    BooleanBuffer::new_set(rows.len())
                } else {
                    BooleanBuffer::new_unset(rows.len())
                }
            },
        ))
    }

    /// Selection bitmap over a contiguous source range: bit `i` is row `rows.start + i`.
    pub(crate) fn select(&self, rows: Range<usize>) -> BooleanBuffer {
        if let Some(bitmap) = &self.bitmap {
            return bitmap(rows);
        }
        let mut builder = BooleanBufferBuilder::new(rows.len());
        let mut scratch = vec![false; rows.len().min(CHUNK)];
        for start in rows.clone().step_by(CHUNK) {
            let end = (start + CHUNK).min(rows.end);
            let out = &mut scratch[..end - start];
            (self.chunk)(RowSelection::Range(start..end), out);
            builder.append_slice(out);
        }
        builder.finish()
    }

    /// Bitwise NOT of this predicate's raw value matches. Callers own null policy.
    pub(crate) fn negate(self) -> Self {
        let Self { chunk, bitmap } = self;
        let bitmap =
            bitmap.map(|bitmap| -> BitmapEvaluator<'a> { Box::new(move |rows| !&bitmap(rows)) });
        let negated = Self::new(Box::new(move |rows, output| {
            chunk(rows, output);
            for value in output.iter_mut() {
                *value = !*value;
            }
        }));
        match bitmap {
            Some(bitmap) => negated.with_bitmap(bitmap),
            None => negated,
        }
    }

    /// Evaluate a single chunk of rows.
    pub(crate) fn evaluate_chunk(&self, rows: RowSelection<'_>, output: &mut [bool]) {
        (self.chunk)(rows, output);
    }

    /// Combine compiled equality alternatives without scanning the source or
    /// materializing one whole-column mask per value.
    pub(crate) fn any_of(predicates: Vec<Self>) -> Self {
        if predicates.is_empty() {
            return Self::empty();
        }
        if predicates.len() == 1 {
            return predicates.into_iter().next().unwrap();
        }
        let predicates = std::sync::Arc::new(predicates);
        let alternatives = predicates.clone();
        Self::new(Box::new(move |rows, output| {
            let mut scratch = vec![false; output.len()];
            for (i, predicate) in predicates.iter().enumerate() {
                if i == 0 {
                    // First alternative directly populates the chunk output with its matches.
                    predicate.evaluate_chunk(rows.clone(), output);
                } else {
                    // Fast circuit-break: once every row matches, skip remaining predicates.
                    if output.iter().all(|&hit| hit) {
                        break;
                    }
                    scratch.fill(false);
                    predicate.evaluate_chunk(rows.clone(), &mut scratch);
                    for (hit, &s) in output.iter_mut().zip(&scratch) {
                        *hit |= s;
                    }
                }
            }
        }))
        .with_bitmap(Box::new(move |rows| {
            let mut bits = alternatives[0].select(rows.clone());
            for predicate in &alternatives[1..] {
                bits |= &predicate.select(rows.clone());
            }
            bits
        }))
    }

    /// Overwrite `matched` with results for the given rows (Range or borrowed slice of row
    /// indices).
    pub(crate) fn evaluate<'r>(
        &self,
        source_rows: impl Into<RowSelection<'r>>,
        matched: &mut [bool],
    ) {
        let source_rows = source_rows.into();
        assert_eq!(source_rows.len(), matched.len());
        for (chunk_idx, out) in matched.chunks_mut(CHUNK).enumerate() {
            let rows = source_rows.chunk(chunk_idx * CHUNK, out.len());
            (self.chunk)(rows, out);
        }
    }
}

/// Pack `test(row)` for every row of a contiguous range, 64 rows per word.
#[inline]
pub(crate) fn collect_bits(
    rows: Range<usize>,
    mut test: impl FnMut(usize) -> bool,
) -> BooleanBuffer {
    let start = rows.start;
    BooleanBuffer::collect_bool(rows.len(), |i| test(start + i))
}

/// Validity bits of `rows`, or `None` when the column has no nulls.
#[inline]
pub(crate) fn validity(nulls: Option<&NullBuffer>, rows: &Range<usize>) -> Option<BooleanBuffer> {
    nulls
        .filter(|nulls| nulls.null_count() > 0)
        .map(|nulls| nulls.inner().slice(rows.start, rows.len()))
}

/// Combine raw value matches with a column's null policy: null rows become
/// `include_nulls`, whatever their value bits held.
#[inline]
pub(crate) fn with_nulls(
    bits: BooleanBuffer,
    nulls: Option<&NullBuffer>,
    rows: &Range<usize>,
    include_nulls: bool,
) -> BooleanBuffer {
    match validity(nulls, rows) {
        None => bits,
        Some(valid) if include_nulls => &bits | &!&valid,
        Some(valid) => &bits & &valid,
    }
}
