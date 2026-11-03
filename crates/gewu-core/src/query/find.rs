//! Cell-level Find over an immutable view and ordered column scope.
//!
//! Find never changes the view or copies Arrow value payload. Directional pages
//! retain bounded cursors, while borrowed row selections and predicate scratch
//! are reused across bounded display-row batches.

use std::{ops::Range, sync::Arc};

use arrow::array::ArrayRef;
use regex::RegexBuilder;

use crate::{
    column::filter::compile_find, execution::predicate::Predicate, Cancellation, Error, Result,
};

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
    /// Source schema column index.
    pub column: u32,
}

/// Cell match flags for a bounded block of display rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindHighlight {
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major flags in the Find session's captured column order.
    pub matched: Vec<bool>,
}

/// A literal matcher over a captured view and ordered column scope.
/// Typed column predicates are compiled and used by each page or highlight request.
///
/// The snapshot shares Arrow buffers and row IDs with its document. Cancellation
/// is terminal and independent of table queries and every other Find session.
pub struct Find {
    columns: Vec<(u32, ArrayRef)>,
    rows: Arc<[u32]>,
    query: String,
    matcher: regex::Regex,
    token: Cancellation,
}

#[derive(Default)]
struct FindScratch {
    source_rows: Vec<usize>,
    matched: Vec<bool>,
    column_hits: Vec<bool>,
}

impl Find {
    pub(crate) fn new(
        columns: Vec<(u32, ArrayRef)>,
        rows: Arc<[u32]>,
        query: String,
    ) -> Result<Self> {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return Err(Error::InvalidArgument("find query cannot be empty".into()));
        }
        let matcher = RegexBuilder::new(&regex::escape(&query))
            .case_insensitive(true)
            .build()
            .map_err(|error| {
                Error::InvalidArgument(format!("cannot compile text search: {error}"))
            })?;
        let token = Cancellation::default();
        Ok(Self {
            columns,
            rows,
            query,
            matcher,
            token,
        })
    }

    /// Permanently cancel this session without affecting any other operation.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Return at most `limit` matches strictly after `after`, in display order.
    /// None starts before the first cell; an empty page marks the end.
    pub fn next(
        &self,
        after: Option<&CellMatch>,
        limit: u32,
        operation: &Cancellation,
    ) -> Result<Vec<CellMatch>> {
        self.scan(after, limit, Direction::Forward, operation)
    }

    /// Return at most `limit` matches strictly before `before`, in display order.
    /// None starts after the last cell; use the first match as the next cursor.
    pub fn previous(
        &self,
        before: Option<&CellMatch>,
        limit: u32,
        operation: &Cancellation,
    ) -> Result<Vec<CellMatch>> {
        self.scan(before, limit, Direction::Backward, operation)
    }

    /// Evaluate a visible block in captured column order with per-request cancellation.
    pub fn highlight(
        &self,
        start: u64,
        count: u32,
        operation: &Cancellation,
    ) -> Result<FindHighlight> {
        let token = self.token.combined_with(operation);
        token.check()?;
        if !(1..=MAX_FIND_HIGHLIGHT_ROWS).contains(&count) {
            return Err(Error::InvalidArgument(format!(
                "find highlight rows must be between 1 and {MAX_FIND_HIGHLIGHT_ROWS}"
            )));
        }
        let predicates = self.predicates()?;
        let len = self.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let mut scratch = FindScratch::default();
        self.evaluate_batch(
            &predicates,
            start as usize..end as usize,
            &token,
            &mut scratch,
        )?;
        Ok(FindHighlight {
            start,
            n_rows: (end - start) as u32,
            n_cols: self.columns.len() as u32,
            matched: scratch.matched,
        })
    }

    fn scan(
        &self,
        cursor: Option<&CellMatch>,
        limit: u32,
        direction: Direction,
        operation: &Cancellation,
    ) -> Result<Vec<CellMatch>> {
        self.token.check()?;
        operation.check()?;
        check_page_limit(limit)?;
        if self.rows.is_empty() || self.columns.is_empty() {
            return Ok(Vec::new());
        }
        let predicates = self.predicates()?;
        self.scan_compiled(cursor, limit, direction, operation, &predicates)
    }

    /// Production execution for compiled predicates. Keeping cancellation
    /// composition here ensures custom and ordinary predicates receive both scopes.
    fn scan_compiled(
        &self,
        cursor: Option<&CellMatch>,
        limit: u32,
        direction: Direction,
        operation: &Cancellation,
        predicates: &[Option<Predicate<'_>>],
    ) -> Result<Vec<CellMatch>> {
        let token = self.token.combined_with(operation);
        token.check()?;
        if self.rows.is_empty() || self.columns.is_empty() {
            return Ok(Vec::new());
        }
        let Some(mut position) = self.start_position(cursor, direction)? else {
            return Ok(Vec::new());
        };
        let n_columns = self.columns.len();
        let n_cells = self.rows.len() * n_columns;
        let mut found = Vec::with_capacity(limit as usize);
        let mut scratch = FindScratch::default();
        while position < n_cells {
            token.check()?;
            let row = position / n_columns;
            let batch = match direction {
                Direction::Forward => row..(row + FIND_BATCH_ROWS).min(self.rows.len()),
                Direction::Backward => (row + 1).saturating_sub(FIND_BATCH_ROWS)..row + 1,
            };
            self.evaluate_batch(predicates, batch.clone(), &token, &mut scratch)?;
            let batch_start = batch.start * n_columns;
            let batch_end = batch.end * n_columns;
            while position >= batch_start && position < batch_end {
                if scratch.matched[position - batch_start] {
                    found.push(CellMatch {
                        display_row: (position / n_columns) as u64,
                        column: self.columns[position % n_columns].0,
                    });
                    if found.len() == limit as usize {
                        token.check()?;
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
        token.check()?;
        if direction == Direction::Backward {
            found.reverse();
        }
        Ok(found)
    }

    fn predicates(&self) -> Result<Vec<Option<Predicate<'_>>>> {
        self.columns
            .iter()
            .map(|(_, array)| compile_find(array, &self.query, Some(&self.matcher)))
            .collect()
    }

    /// Convert only this bounded batch of row IDs. Predicates borrow the original
    /// arrays and reuse their own bounded masks/formatters for selected cells.
    fn evaluate_batch(
        &self,
        predicates: &[Option<Predicate<'_>>],
        display_rows: Range<usize>,
        token: &Cancellation,
        scratch: &mut FindScratch,
    ) -> Result<()> {
        token.check()?;
        scratch.source_rows.clear();
        scratch
            .source_rows
            .extend(self.rows[display_rows].iter().map(|&row| row as usize));
        let n_rows = scratch.source_rows.len();
        let n_columns = self.columns.len();
        scratch.matched.clear();
        scratch.matched.resize(n_rows * n_columns, false);
        scratch.column_hits.clear();
        scratch.column_hits.resize(n_rows, false);
        for (column, predicate) in predicates.iter().enumerate() {
            token.check()?;
            if let Some(predicate) = predicate {
                predicate.evaluate(&scratch.source_rows, &mut scratch.column_hits, token)?;
            } else {
                scratch.column_hits.fill(false);
            }
            for (row, hit) in scratch.column_hits.iter().copied().enumerate() {
                scratch.matched[row * n_columns + column] = hit;
            }
        }
        token.check()
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
        let row = usize::try_from(cursor.display_row).map_err(|_| {
            Error::InvalidArgument("find cursor row is outside the captured view".into())
        })?;
        let column = self
            .columns
            .iter()
            .position(|(column, _)| *column == cursor.column)
            .ok_or_else(|| {
                Error::InvalidArgument("find cursor column is outside the captured scope".into())
            })?;
        if row >= self.rows.len() {
            return Err(Error::InvalidArgument(
                "find cursor row is outside the captured view".into(),
            ));
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
        Err(Error::InvalidArgument(format!(
            "find page size must be between 1 and {MAX_FIND_PAGE_SIZE}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::StringArray;

    use super::*;

    #[test]
    fn session_cancellation_reaches_a_running_inner_predicate() {
        running_inner_scan_is_cancelled(true);
    }

    #[test]
    fn operation_cancellation_reaches_a_running_inner_predicate() {
        running_inner_scan_is_cancelled(false);
    }

    fn running_inner_scan_is_cancelled(session: bool) {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        let array: ArrayRef = Arc::new(StringArray::from(vec!["value"]));
        let find = Find::new(vec![(0, array)], Arc::from([0]), "value".into()).unwrap();
        let operation = Cancellation::default();
        let (started, start) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let resumed = std::sync::Mutex::new(resumed);
        let observed = Arc::new(std::sync::Mutex::new(false));
        let inner_observed = observed.clone();
        let predicates = vec![Some(Predicate::new(Box::new(move |_, _, _, cancel| {
            started.send(()).unwrap();
            resumed.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
            let result = cancel.check();
            *inner_observed.lock().unwrap() = matches!(result, Err(Error::Cancelled));
            result
        })))];
        std::thread::scope(|scope| {
            let worker = scope
                .spawn(|| find.scan_compiled(None, 1, Direction::Forward, &operation, &predicates));
            start.recv_timeout(TIMEOUT).unwrap();
            if session {
                find.cancel();
            } else {
                operation.cancel();
            }
            resume.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
        });
        assert!(
            *observed.lock().unwrap(),
            "inner predicate must observe cancellation"
        );
    }
}
