//! Snapshot-owned cell Find: wrapping navigation and highlights.

use gewu_core::{self as core, CellMatch, ColumnHighlight, ColumnId, FindHighlight};

use crate::GewuError;

/// A matching cell in display order.
#[uniffi::remote(Record)]
pub struct CellMatch {
    pub display_row: u64,
    pub column: ColumnId,
}

/// Match flags for a block of display rows, one bitmap per requested column.
#[uniffi::remote(Record)]
pub struct FindHighlight {
    pub start: u64,
    pub n_rows: u32,
    /// One entry per requested column, in request order.
    pub columns: Vec<ColumnHighlight>,
}

/// Match flags for one column of a [`FindHighlight`].
#[uniffi::remote(Record)]
pub struct ColumnHighlight {
    pub column: ColumnId,
    /// One bit per row starting at the block's first row, low bit first.
    pub matched: Vec<u8>,
}

/// Cell Find over one ordered view: a compiled query and an ordered column
/// scope, bound to the rows the view had when it was created.
#[derive(uniffi::Object)]
pub struct GewuFind {
    pub(crate) inner: core::Find,
}

#[uniffi::export]
impl GewuFind {
    /// The match `by` matches after `from` (before it when negative), in
    /// display order, wrapping around at either end. `from == None` starts
    /// outside the cells, so stepping by 1 finds the first match and by -1
    /// the last. Returns `None` when nothing matches, and `from` when `by` is 0.
    /// Scans only as far as the result.
    pub fn step(&self, from: Option<CellMatch>, by: i64) -> Result<Option<CellMatch>, GewuError> {
        Ok(self.step_inner(from, by)?)
    }

    /// Match flags for the requested columns, such as those on screen, over
    /// display rows `start..start + count`, clipped to the available rows.
    /// Every column must belong to the captured scope.
    pub fn highlight(
        &self,
        columns: Vec<ColumnId>,
        start: u64,
        count: u32,
    ) -> Result<FindHighlight, GewuError> {
        Ok(self.highlight_inner(&columns, start, count)?)
    }
}

impl GewuFind {
    fn step_inner(
        &self,
        from: Option<CellMatch>,
        by: i64,
    ) -> Result<Option<CellMatch>, core::FindError> {
        if by == 0 {
            return Ok(from);
        }
        let forward = by > 0;
        let mut remaining = by.unsigned_abs();
        let mut cursor = from;
        // Matches passed since the last start from outside the cells; once a
        // whole pass is known, the remaining steps reduce modulo its length.
        let mut passed: Option<u64> = from.is_none().then_some(0);
        loop {
            let limit = remaining.min(u64::from(core::MAX_FIND_PAGE_SIZE)) as u32;
            let mut page = if forward {
                self.inner.next(cursor.as_ref(), limit)?
            } else {
                self.inner.previous(cursor.as_ref(), limit)?
            };
            if page.is_empty() {
                match passed {
                    Some(0) => return Ok(None),
                    Some(total) => remaining = (remaining - 1) % total + 1,
                    None => {}
                }
                cursor = None;
                passed = Some(0);
                continue;
            }
            if !forward {
                page.reverse();
            }
            let taken = page.len() as u64;
            if remaining <= taken {
                return Ok(Some(page[remaining as usize - 1]));
            }
            remaining -= taken;
            cursor = page.last().copied();
            if let Some(passed) = &mut passed {
                *passed += taken;
            }
        }
    }

    fn highlight_inner(
        &self,
        columns: &[ColumnId],
        start: u64,
        count: u32,
    ) -> Result<FindHighlight, core::FindError> {
        let chunk = core::MAX_FIND_HIGHLIGHT_ROWS;
        let mut result = FindHighlight {
            start,
            n_rows: 0,
            columns: columns
                .iter()
                .map(|&column| ColumnHighlight {
                    column,
                    matched: Vec::new(),
                })
                .collect(),
        };
        let mut offset = 0;
        while offset < count {
            let block = self.inner.highlight(
                columns,
                start + u64::from(offset),
                chunk.min(count - offset),
            )?;
            if offset == 0 {
                result.start = block.start;
            }
            if block.n_rows == 0 {
                break;
            }
            // Blocks other than the last cover whole bytes, so bitmaps concatenate.
            for (all, part) in result.columns.iter_mut().zip(block.columns) {
                all.matched.extend(part.matched);
            }
            result.n_rows += block.n_rows;
            offset += block.n_rows;
            if block.n_rows < chunk {
                break;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
