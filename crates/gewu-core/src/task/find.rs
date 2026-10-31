//! Cell-level Find over an immutable view and ordered column scope.
//!
//! Find never changes the view or copies Arrow value payload. Directional pages
//! retain bounded cursors, while borrowed row selections and predicate scratch
//! are reused across bounded display-row batches.

use std::{
    ops::Range,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use arrow::array::ArrayRef;

use crate::{
    column::{Column, SearchQuery},
    engine::CancelToken,
    predicate::{Predicate, RowSelection},
    Error, Result,
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
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major flags in the Find session's captured column order.
    pub matched: Vec<bool>,
}

/// Cancellation for one page or highlight scan, independent of its Find session.
/// Cancelling an obsolete scan does not close the session or affect newer scans.
pub struct FindOperation {
    cancellation: Arc<AtomicU64>,
    token: CancelToken,
}

impl FindOperation {
    pub fn cancel(&self) {
        self.cancellation.fetch_add(1, Ordering::SeqCst);
    }
}

/// A prepared query over a captured view and ordered column scope.
///
/// The snapshot shares Arrow buffers and row IDs with its document. Cancellation
/// is terminal and independent of table queries and every other Find session.
pub struct Find {
    columns: Vec<(u32, ArrayRef)>,
    rows: Arc<Vec<u32>>,
    version: u64,
    query: SearchQuery,
    cancellation: Arc<AtomicU64>,
    token: CancelToken,
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
        rows: Arc<Vec<u32>>,
        version: u64,
        query: String,
    ) -> Result<Self> {
        let query = SearchQuery::new(query.trim());
        if query.text().is_empty() {
            return Err(Error::InvalidArgument("find query cannot be empty".into()));
        }
        for (_, array) in &columns {
            drop(Column::new(array).prepare_search(&query)?);
        }
        let (token, cancellation) = CancelToken::independent();
        Ok(Self {
            columns,
            rows,
            version,
            query,
            cancellation,
            token,
        })
    }

    /// Permanently cancel this session without affecting any other operation.
    pub fn cancel(&self) {
        self.cancellation.fetch_add(1, Ordering::SeqCst);
    }

    /// Create independently cancellable work for one page or highlight request.
    pub fn operation(&self) -> FindOperation {
        let (token, cancellation) = CancelToken::independent();
        FindOperation {
            cancellation,
            token,
        }
    }

    /// Return at most `limit` matches strictly after `after`, in display order.
    /// `None` starts before the first cell. An empty page means the end was reached;
    /// callers may wrap by calling again with `None`.
    pub fn next(&self, after: Option<&CellMatch>, limit: u32) -> Result<Vec<CellMatch>> {
        self.scan(after, limit, Direction::Forward, &self.token)
    }

    /// [`Self::next`] with cancellation scoped to this request only.
    pub fn next_with_operation(
        &self,
        after: Option<&CellMatch>,
        limit: u32,
        operation: &FindOperation,
    ) -> Result<Vec<CellMatch>> {
        self.scan(after, limit, Direction::Forward, &operation.token)
    }

    /// Return at most `limit` matches strictly before `before`, in display order.
    /// `None` starts after the last cell. Use the first returned match as the
    /// continuation cursor for another backwards page. An empty page means the
    /// beginning was reached; callers may wrap by calling again with `None`.
    pub fn previous(&self, before: Option<&CellMatch>, limit: u32) -> Result<Vec<CellMatch>> {
        self.scan(before, limit, Direction::Backward, &self.token)
    }

    /// [`Self::previous`] with cancellation scoped to this request only.
    pub fn previous_with_operation(
        &self,
        before: Option<&CellMatch>,
        limit: u32,
        operation: &FindOperation,
    ) -> Result<Vec<CellMatch>> {
        self.scan(before, limit, Direction::Backward, &operation.token)
    }

    /// Evaluate the same typed query for a visible block without formatting cells.
    /// The result uses this session's captured column order and remains valid if
    /// the document later publishes another view.
    pub fn highlight(&self, start: u64, count: u32) -> Result<FindHighlight> {
        self.highlight_with_token(start, count, &self.token)
    }

    /// [`Self::highlight`] with cancellation scoped to this request only.
    pub fn highlight_with_operation(
        &self,
        start: u64,
        count: u32,
        operation: &FindOperation,
    ) -> Result<FindHighlight> {
        self.highlight_with_token(start, count, &operation.token)
    }

    fn highlight_with_token(
        &self,
        start: u64,
        count: u32,
        operation: &CancelToken,
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
            version: self.version,
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
        operation: &CancelToken,
    ) -> Result<Vec<CellMatch>> {
        self.token.check()?;
        operation.check()?;
        check_page_limit(limit)?;
        if self.rows.is_empty() || self.columns.is_empty() {
            return Ok(Vec::new());
        }
        let predicates = self.predicates()?;
        self.scan_prepared(cursor, limit, direction, operation, &predicates)
    }

    /// Production execution for prepared predicates. Keeping cancellation
    /// composition here ensures custom and ordinary predicates receive both scopes.
    fn scan_prepared(
        &self,
        cursor: Option<&CellMatch>,
        limit: u32,
        direction: Direction,
        operation: &CancelToken,
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
            .map(|(_, array)| Column::new(array).prepare_search(&self.query))
            .collect()
    }

    /// Convert only this bounded batch of row IDs. Predicates borrow the original
    /// arrays and reuse their own bounded masks/formatters for selected cells.
    fn evaluate_batch(
        &self,
        predicates: &[Option<Predicate<'_>>],
        display_rows: Range<usize>,
        token: &CancelToken,
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
                predicate.evaluate_selected(
                    RowSelection::Indices(&scratch.source_rows),
                    &mut scratch.column_hits,
                    token,
                )?;
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
        let find = Find::new(vec![(0, array)], Arc::new(vec![0]), 0, "value".into()).unwrap();
        let operation = find.operation();
        let (started, start) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let resumed = std::sync::Mutex::new(resumed);
        let observed = Arc::new(std::sync::Mutex::new(false));
        let inner_observed = observed.clone();
        let predicates = vec![Some(Predicate::batch(Box::new(move |_, _, cancel| {
            started.send(()).unwrap();
            resumed.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
            let result = cancel.check();
            *inner_observed.lock().unwrap() = matches!(result, Err(Error::Cancelled));
            result
        })))];
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                find.scan_prepared(None, 1, Direction::Forward, &operation.token, &predicates)
            });
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
