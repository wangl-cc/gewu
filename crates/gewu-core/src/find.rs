//! Cell-level Find over an immutable view and ordered column scope.
//!
//! Find never changes the view or copies Arrow value payload. Directional pages
//! retain bounded cursors, while borrowed row selections and predicate scratch
//! are reused across bounded display-row batches.

mod error;

use std::ops::Range;

use arrow::array::ArrayRef;
pub use error::FindError;
use regex::RegexBuilder;

use crate::{
    column::predicate::compile_find, execution::predicate::Predicate, row_ids::RowIds, ColumnId,
};

type Result<T> = std::result::Result<T, FindError>;

/// Maximum number of cell cursors returned by one directional Find request.
pub const MAX_FIND_PAGE_SIZE: u32 = 256;
/// Maximum number of display rows covered by one highlight request.
pub const MAX_FIND_HIGHLIGHT_ROWS: u32 = 256;
const FIND_BATCH_ROWS: usize = 1024;

/// A matching cell in display order. This is also an exclusive continuation cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellMatch {
    /// Zero-based row offset in the captured view, not a source row number.
    pub display_row: u64,
    /// Source schema column ID.
    pub column: ColumnId,
}

/// Match flags for a bounded block of display rows, one bitmap per requested column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindHighlight {
    pub start: u64,
    pub n_rows: u32,
    /// One entry per requested column, in request order.
    pub columns: Vec<ColumnHighlight>,
}

/// Match flags for one column of a [`FindHighlight`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnHighlight {
    pub column: ColumnId,
    /// One bit per row starting at the block's first row, low bit first.
    pub matched: Vec<u8>,
}

/// A literal matcher over a captured view and ordered column scope.
/// Typed column predicates are compiled and used by each page or highlight request.
///
/// The snapshot shares Arrow buffers and row IDs with its document.
pub struct Find {
    columns: Vec<(ColumnId, ArrayRef)>,
    rows: RowIds,
    query: String,
    matcher: regex::Regex,
}

#[derive(Default)]
struct FindScratch {
    source_rows: Vec<usize>,
    matched: Vec<bool>,
    column_hits: Vec<bool>,
}

impl Find {
    pub(crate) fn new(
        columns: Vec<(ColumnId, ArrayRef)>,
        rows: RowIds,
        query: String,
    ) -> Result<Self> {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return Err(crate::FindError::EmptyQuery);
        }
        let matcher = RegexBuilder::new(&regex::escape(&query))
            .case_insensitive(true)
            .build()
            .map_err(|source| crate::FindError::QueryCompilationFailed { source })?;
        Ok(Self {
            columns,
            rows,
            query,
            matcher,
        })
    }

    /// Return at most `limit` matches strictly after `after`, in display order.
    /// None starts before the first cell; an empty page marks the end.
    pub fn next(&self, after: Option<&CellMatch>, limit: u32) -> Result<Vec<CellMatch>> {
        self.scan(after, limit, Direction::Forward)
    }

    /// Return at most `limit` matches strictly before `before`, in display order.
    /// None starts after the last cell; use the first match as the next cursor.
    pub fn previous(&self, before: Option<&CellMatch>, limit: u32) -> Result<Vec<CellMatch>> {
        self.scan(before, limit, Direction::Backward)
    }

    /// Evaluate only the requested columns, such as those on screen, over a
    /// bounded block of display rows. Every column must belong to the captured
    /// scope; the block is clipped to the available rows.
    pub fn highlight(&self, columns: &[ColumnId], start: u64, count: u32) -> Result<FindHighlight> {
        if !(1..=MAX_FIND_HIGHLIGHT_ROWS).contains(&count) {
            return Err(crate::FindError::InvalidHighlightRows {
                value: count,
                min: 1,
                max: MAX_FIND_HIGHLIGHT_ROWS,
            });
        }
        let arrays = columns
            .iter()
            .map(|&column| {
                self.columns
                    .iter()
                    .find(|(id, _)| *id == column)
                    .map(|(_, array)| array)
                    .ok_or(crate::FindError::InvalidHighlightColumn { column })
            })
            .collect::<Result<Vec<_>>>()?;
        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let window = self.rows.as_slice().window(start as usize, end as usize);
        let n_rows = window.len();
        let contiguous = window.contiguous_range();
        let source_rows: Vec<usize> = if contiguous.is_none() {
            window.iter().collect()
        } else {
            Vec::new()
        };
        let mut hits = vec![false; n_rows];
        let mut result = Vec::with_capacity(columns.len());
        for (&column, array) in columns.iter().zip(arrays) {
            let predicate = compile_find(array, &self.query, Some(&self.matcher))?;
            match (&predicate, contiguous.clone()) {
                (Some(predicate), Some(range)) => predicate.evaluate(range, &mut hits),
                (Some(predicate), None) => predicate.evaluate(&source_rows, &mut hits),
                (None, _) => hits.fill(false),
            }
            let mut matched = vec![0u8; n_rows.div_ceil(8)];
            for (row, _) in hits.iter().enumerate().filter(|(_, hit)| **hit) {
                matched[row / 8] |= 1 << (row % 8);
            }
            result.push(ColumnHighlight { column, matched });
        }
        Ok(FindHighlight {
            start,
            n_rows: n_rows as u32,
            columns: result,
        })
    }

    fn scan(
        &self,
        cursor: Option<&CellMatch>,
        limit: u32,
        direction: Direction,
    ) -> Result<Vec<CellMatch>> {
        check_page_limit(limit)?;
        if self.rows.is_empty() || self.columns.is_empty() {
            return Ok(Vec::new());
        }
        let predicates = self.predicates()?;
        let Some(mut position) = self.start_position(cursor, direction)? else {
            return Ok(Vec::new());
        };
        let n_columns = self.columns.len();
        let n_cells = self.rows.len() * n_columns;
        let mut found = Vec::with_capacity(limit as usize);
        let mut scratch = FindScratch::default();
        while position < n_cells {
            let row = position / n_columns;
            let batch = match direction {
                Direction::Forward => row..(row + FIND_BATCH_ROWS).min(self.rows.len()),
                Direction::Backward => (row + 1).saturating_sub(FIND_BATCH_ROWS)..row + 1,
            };
            self.evaluate_batch(&predicates, batch.clone(), &mut scratch);
            let batch_start = batch.start * n_columns;
            let batch_end = batch.end * n_columns;
            while position >= batch_start && position < batch_end {
                if scratch.matched[position - batch_start] {
                    found.push(CellMatch {
                        display_row: (position / n_columns) as u64,
                        column: self.columns[position % n_columns].0,
                    });
                    if found.len() == limit as usize {
                        if direction == Direction::Backward {
                            found.reverse();
                        }
                        return Ok(found);
                    }
                }
                position = match direction {
                    Direction::Forward => position + 1,
                    Direction::Backward if position > 0 => position - 1,
                    Direction::Backward => n_cells,
                };
            }
        }
        if direction == Direction::Backward {
            found.reverse();
        }
        Ok(found)
    }

    fn predicates(&self) -> Result<Vec<Option<Predicate<'_>>>> {
        self.columns
            .iter()
            .map(|(_, array)| {
                compile_find(array, &self.query, Some(&self.matcher)).map_err(FindError::from)
            })
            .collect()
    }

    /// Convert only this bounded batch of row IDs. Predicates borrow the original
    /// arrays and reuse their own bounded masks/formatters for selected cells.
    fn evaluate_batch(
        &self,
        predicates: &[Option<Predicate<'_>>],
        display_rows: Range<usize>,
        scratch: &mut FindScratch,
    ) {
        let window = self
            .rows
            .as_slice()
            .window(display_rows.start, display_rows.end);
        let n_rows = window.len();
        let n_columns = self.columns.len();
        scratch.matched.clear();
        scratch.matched.resize(n_rows * n_columns, false);
        scratch.column_hits.clear();
        scratch.column_hits.resize(n_rows, false);
        let contiguous = window.contiguous_range();
        if contiguous.is_none() {
            scratch.source_rows.clear();
            scratch.source_rows.extend(window.iter());
        }
        for (column, predicate) in predicates.iter().enumerate() {
            if let Some(predicate) = predicate {
                if let Some(range) = contiguous.clone() {
                    predicate.evaluate(range, &mut scratch.column_hits);
                } else {
                    predicate.evaluate(&scratch.source_rows, &mut scratch.column_hits);
                }
            } else {
                scratch.column_hits.fill(false);
            }
            for (row, hit) in scratch.column_hits.iter().copied().enumerate() {
                scratch.matched[row * n_columns + column] = hit;
            }
        }
    }

    fn start_position(
        &self,
        cursor: Option<&CellMatch>,
        direction: Direction,
    ) -> Result<Option<usize>> {
        let n_columns = self.columns.len();
        let Some(cursor) = cursor else {
            return Ok(Some(match direction {
                Direction::Forward => 0,
                Direction::Backward => self.rows.len() * n_columns - 1,
            }));
        };
        let invalid_row = || crate::FindError::InvalidCursorRow {
            row: cursor.display_row,
            row_count: self.rows.len() as u64,
        };
        let row = usize::try_from(cursor.display_row).map_err(|_| invalid_row())?;
        let column = self
            .columns
            .iter()
            .position(|(column, _)| *column == cursor.column)
            .ok_or_else(|| crate::FindError::InvalidCursorColumn {
                column: cursor.column,
            })?;
        if row >= self.rows.len() {
            return Err(invalid_row());
        }
        let position = row * n_columns + column;
        Ok(match direction {
            Direction::Forward => Some(position + 1),
            Direction::Backward if position > 0 => Some(position - 1),
            Direction::Backward => None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Backward,
}

fn check_page_limit(limit: u32) -> Result<()> {
    if (1..=MAX_FIND_PAGE_SIZE).contains(&limit) {
        Ok(())
    } else {
        Err(crate::FindError::InvalidPageSize {
            value: limit,
            min: 1,
            max: MAX_FIND_PAGE_SIZE,
        })
    }
}
