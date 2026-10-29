//! Prepared, column-bound predicates. Preparation does not scan rows.
//!
//! Row predicates retain native typed comparisons and can short-circuit an
//! AND. Batch predicates handle formatted fallback or nested arrays. All row numbers
//! are source offsets; output slot zero corresponds to `source_rows.start`.

use std::ops::Range;

use crate::{engine::CancelToken, Result};

pub(crate) const CHUNK: usize = 64 * 1024;
pub(crate) type RowTest<'a> = Box<dyn Fn(usize) -> bool + Send + Sync + 'a>;
type BatchTest<'a> =
    Box<dyn Fn(Range<usize>, &mut [bool], &CancelToken) -> Result<()> + Send + Sync + 'a>;

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

    /// Overwrite the output with matches for this source range.
    pub(crate) fn evaluate(
        &self,
        source_rows: Range<usize>,
        matched: &mut [bool],
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
                        *hit = test(row);
                    }
                }
            }
            Self::Batch(test) => {
                for (chunk, out) in matched.chunks_mut(CHUNK).enumerate() {
                    cancel.check()?;
                    let start = source_rows.start + chunk * CHUNK;
                    out.fill(false);
                    test(start..start + out.len(), out, cancel)?;
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
