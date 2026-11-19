//! Compiled, column-bound predicates for high-throughput query execution.
//!
//! Two consumers, two shapes:
//! - **Filters** compile to a [`Selector`]: one packed [`BooleanBuffer`] over a contiguous source
//!   range. Null policy, exclusion and alternatives combine bitmaps with bitwise operations.
//! - **Find** evaluates display-ordered row IDs with a [`Predicate`]'s chunk evaluator in 8K chunks
//!   (`CHUNK = 8 * 1024`), so working data and masks stay in the L1 data cache.
//!
//! Leaf predicates (ranges, text, scalars) serve both. Hot leaves also carry a native bitmap
//! evaluator that compares 64 rows per word; the others are packed from their chunk evaluator.

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
        Self::new(Box::new(|_rows, output| output.fill(false)))
            .with_bitmap(Box::new(|rows| BooleanBuffer::new_unset(rows.len())))
    }

    /// Selection bitmap over a contiguous source range: bit `i` is row `rows.start + i`.
    pub(crate) fn select(&self, rows: Range<usize>) -> BooleanBuffer {
        match &self.bitmap {
            Some(bitmap) => bitmap(rows),
            None => self.pack_chunks(rows),
        }
    }

    /// Selection bitmap packed from the chunk evaluator.
    pub(crate) fn pack_chunks(&self, rows: Range<usize>) -> BooleanBuffer {
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

/// A compiled filter condition. Filters only ever need whole-range selection
/// bitmaps, so composition (null policy, exclusion, alternatives) happens on
/// bitmaps; leaf [`Predicate`]s, shared with Find, supply them.
pub(crate) struct Selector<'a>(BitmapEvaluator<'a>);

impl<'a> Selector<'a> {
    pub(crate) fn new(bitmap: BitmapEvaluator<'a>) -> Self {
        Self(bitmap)
    }

    /// Select every row (`true`) or none (`false`).
    pub(crate) fn constant(hit: bool) -> Self {
        Self::new(Box::new(move |rows| {
            if hit {
                BooleanBuffer::new_set(rows.len())
            } else {
                BooleanBuffer::new_unset(rows.len())
            }
        }))
    }

    /// Selection bitmap over a contiguous source range: bit `i` is row `rows.start + i`.
    pub(crate) fn select(&self, rows: Range<usize>) -> BooleanBuffer {
        (self.0)(rows)
    }

    /// Bitwise NOT of the selection. Callers own null policy.
    pub(crate) fn negate(self) -> Self {
        Self::new(Box::new(move |rows| !&self.select(rows)))
    }

    /// Rows selected by any alternative.
    pub(crate) fn any_of(alternatives: Vec<Self>) -> Self {
        Self::new(Box::new(move |rows| {
            let mut bits = BooleanBuffer::new_unset(rows.len());
            for alternative in &alternatives {
                bits |= &alternative.select(rows.clone());
            }
            bits
        }))
    }

    /// Overwrite `matched` with the selection over `rows`.
    #[cfg(test)]
    pub(crate) fn evaluate(&self, rows: Range<usize>, matched: &mut [bool]) {
        assert_eq!(rows.len(), matched.len());
        for (hit, bit) in matched.iter_mut().zip(self.select(rows).iter()) {
            *hit = bit;
        }
    }
}

impl<'a> From<Predicate<'a>> for Selector<'a> {
    fn from(predicate: Predicate<'a>) -> Self {
        Self::new(Box::new(move |rows| predicate.select(rows)))
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
