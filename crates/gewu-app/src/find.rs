//! Find over one displayed view: the search, navigation between matching cells, the match
//! count and the highlight cache for the visible rows.
//!
//! [`FindSession`] holds the state and decides which background work is current. Each edit
//! returns a [`FindWork`] with the jobs to run away from the UI thread; their results come
//! back through the `*_finished` methods, which discard results of a search that has since
//! been replaced. The caller owns the tasks, the text input and the timing of typing.
use std::{
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use gewu_core::{
    CellMatch, ColumnHighlight, ColumnId, Error, Find, FindCounts, FindError, FindHighlight,
    OrderedView, ViewRow, MAX_FIND_HIGHLIGHT_ROWS, MAX_FIND_PAGE_SIZE,
};

/// Rows per highlight cache block. Reads cover whole blocks so that scrolling within a block
/// needs no new read.
pub const HIGHLIGHT_BLOCK_ROWS: u64 = 64;
/// Cached blocks beyond this many screens are dropped, so a long jump through a large file
/// does not grow the cache with the rows it passed.
const CACHED_SCREENS: usize = 4;

/// Navigate a signed number of matching cells, paging within Core's limit and wrapping
/// at either end. The flag records whether navigation passed the end of the match list.
pub fn step(
    find: &Find,
    from: Option<CellMatch>,
    by: i64,
) -> Result<(Option<CellMatch>, bool), FindError> {
    if by == 0 {
        return Ok((from, false));
    }
    let forward = by > 0;
    let mut wrapped = false;
    let mut remaining = by.unsigned_abs();
    let mut cursor = from;
    // Matches passed since the last start from outside the cells; once a
    // whole pass is known, the remaining steps reduce modulo its length.
    let mut passed: Option<u64> = from.is_none().then_some(0);
    loop {
        let limit = remaining.min(u64::from(MAX_FIND_PAGE_SIZE)) as u32;
        let mut page = if forward {
            find.next(cursor.as_ref(), limit)?
        } else {
            find.previous(cursor.as_ref(), limit)?
        };
        if page.is_empty() {
            match passed {
                Some(0) => return Ok((None, false)),
                Some(total) => remaining = (remaining - 1) % total + 1,
                None => {}
            }
            wrapped = true;
            cursor = None;
            passed = Some(0);
            continue;
        }
        if !forward {
            page.reverse();
        }
        let taken = page.len() as u64;
        if remaining <= taken {
            return Ok((Some(page[remaining as usize - 1]), wrapped));
        }
        remaining -= taken;
        cursor = page.last().copied();
        if let Some(passed) = &mut passed {
            *passed += taken;
        }
    }
}

/// What the search shows.
pub enum FindStatus {
    Idle,
    /// A step is running from the current match, if any.
    Searching {
        from: Option<CellMatch>,
    },
    At {
        hit: CellMatch,
        wrapped: bool,
    },
    NoMatches,
    /// Searching, stepping, counting or reading highlights failed. `at` keeps the match
    /// shown before.
    Failed {
        error: Error,
        at: Option<CellMatch>,
    },
}

impl FindStatus {
    fn current(&self) -> Option<CellMatch> {
        match self {
            Self::Searching { from: at } | Self::Failed { at, .. } => *at,
            Self::At { hit, .. } => Some(*hit),
            Self::Idle | Self::NoMatches => None,
        }
    }
}

enum Search {
    Unstarted,
    Ready(Arc<Find>),
    /// Creating the search failed; it is not retried until the scope changes.
    Failed,
}

enum Navigation {
    Idle,
    /// Steps requested while one runs add up and run next.
    Running {
        queued: i64,
    },
}

enum HighlightLoad {
    Idle,
    Reading,
    /// A read failed; highlights stop until the scope changes.
    Failed,
}

struct Count {
    // Navigation can move and wrap while the tally for its starting cell is still running.
    start: Option<FindCounts>,
    movement: i128,
    cancel: Arc<AtomicBool>,
}

impl Drop for Count {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Matching cells per (column, block), each sharing the bitmap of the read that found them.
#[derive(Default)]
struct Highlights(HashMap<(ColumnId, u64), (u64, Rc<ColumnHighlight>)>);

impl Highlights {
    fn cached_match(&self, column: ColumnId, row: u64) -> Option<bool> {
        self.0
            .get(&(column, row / HIGHLIGHT_BLOCK_ROWS))
            .map(|(start, column)| column.matches((row - start) as usize))
    }

    fn store(&mut self, result: FindHighlight) {
        let first = result.start / HIGHLIGHT_BLOCK_ROWS;
        let count = u64::from(result.n_rows).div_ceil(HIGHLIGHT_BLOCK_ROWS);
        for column in result.columns {
            // A batched read can fill several cache slots with the same bitmap.
            let column = Rc::new(column);
            for block in first..first + count {
                self.0
                    .insert((column.column, block), (result.start, Rc::clone(&column)));
            }
        }
    }
}

/// A step through the matches, to run in the background.
pub struct StepJob {
    find: Arc<Find>,
    from: Option<CellMatch>,
    by: i64,
    /// Wait for typing to pause before searching.
    pub debounce: bool,
}

pub struct StepResult {
    find: Arc<Find>,
    from: Option<CellMatch>,
    by: i64,
    result: Result<(Option<CellMatch>, bool), FindError>,
}

impl StepJob {
    pub fn run(self) -> StepResult {
        StepResult {
            result: step(&self.find, self.from, self.by),
            find: self.find,
            from: self.from,
            by: self.by,
        }
    }
}

/// Counting the matches up to a hit, cancelled when the count is no longer shown.
pub struct CountJob {
    find: Arc<Find>,
    hit: CellMatch,
    cancel: Arc<AtomicBool>,
}

pub struct CountResult {
    cancel: Arc<AtomicBool>,
    result: Result<Option<FindCounts>, FindError>,
}

impl CountJob {
    pub fn run(self) -> CountResult {
        let cancel = &self.cancel;
        CountResult {
            result: self
                .find
                .counts_at(self.hit, || cancel.load(Ordering::Relaxed)),
            cancel: self.cancel,
        }
    }
}

/// Reading which visible cells match, for a run of uncached blocks.
pub struct HighlightJob {
    find: Arc<Find>,
    columns: Vec<ColumnId>,
    start: u64,
    rows: u32,
}

pub struct HighlightResult {
    find: Arc<Find>,
    result: Result<FindHighlight, FindError>,
}

impl HighlightJob {
    pub fn run(self) -> HighlightResult {
        HighlightResult {
            result: self.find.highlight(&self.columns, self.start, self.rows),
            find: self.find,
        }
    }
}

/// The work an edit asks for. A present job supersedes the running job of its kind.
#[derive(Default)]
pub struct FindWork {
    pub step: Option<StepJob>,
    pub count: Option<CountJob>,
    pub highlight: Option<HighlightJob>,
    /// The match to select and reveal in the table.
    pub reveal: Option<CellMatch>,
}

/// Find in one table: open while the search field is in use.
pub struct FindSession {
    open: bool,
    /// The search text, taken from the input on each edit.
    text: String,
    /// The view searched; `None` while a new query is applied.
    view: Option<Arc<OrderedView>>,
    /// Visible columns in display order.
    columns: Vec<ColumnId>,
    search: Search,
    status: FindStatus,
    navigation: Navigation,
    count: Option<Count>,
    highlights: Highlights,
    highlight_load: HighlightLoad,
    viewport_rows: Range<u64>,
    viewport_columns: Vec<ColumnId>,
}

impl FindSession {
    pub fn new(view: Arc<OrderedView>, columns: Vec<ColumnId>) -> Self {
        Self {
            open: false,
            text: String::new(),
            view: Some(view),
            columns,
            search: Search::Unstarted,
            status: FindStatus::Idle,
            navigation: Navigation::Idle,
            count: None,
            highlights: Highlights::default(),
            highlight_load: HighlightLoad::Idle,
            viewport_rows: 0..0,
            viewport_columns: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn status(&self) -> &FindStatus {
        &self.status
    }

    pub fn view(&self) -> Option<&Arc<OrderedView>> {
        self.view.as_ref()
    }

    /// The match the table outlines.
    pub fn current(&self) -> Option<CellMatch> {
        self.status.current()
    }

    pub fn can_navigate(&self) -> bool {
        matches!(self.navigation, Navigation::Running { .. }) || self.current().is_some()
    }

    /// The current match's position among all matches and their total, once counted.
    /// `Some(None)` while counting.
    pub fn count(&self) -> Option<Option<(u64, u64)>> {
        let count = self.count.as_ref()?;
        let Some(FindCounts { through, total }) = count.start else {
            return Some(None);
        };
        if through == 0 || total == 0 {
            return None;
        }
        let zero_based = (i128::from(through - 1) + count.movement).rem_euclid(i128::from(total));
        Some(Some((zero_based as u64 + 1, total)))
    }

    /// `None` means the background search has not classified this cell yet.
    pub fn cached_match(&self, column: ColumnId, row: u64) -> Option<bool> {
        self.highlights.cached_match(column, row)
    }

    /// Text ranges to highlight in a displayed cell. `preview` is the single-line prefix of
    /// `source` the grid shows, followed by an ellipsis when it is shorter, and `shown_len`
    /// the length of the shown text including that ellipsis. `source` is read only when the
    /// cell may match.
    pub fn cell_spans<'a>(
        &self,
        column: ColumnId,
        row: ViewRow,
        text_matchable: bool,
        source: impl FnOnce() -> Option<(&'a str, &'a str)>,
        shown_len: usize,
        spans: &mut Vec<Range<usize>>,
    ) {
        let Search::Ready(find) = &self.search else {
            return;
        };
        let hit = if self.current()
            == Some(CellMatch {
                display_row: row,
                column,
            }) {
            Some(true)
        } else {
            self.cached_match(column, row.0)
        };
        if !hit.unwrap_or(text_matchable) {
            return;
        }
        let Some((source, preview)) = source() else {
            return;
        };
        // A cold viewport can immediately prove literal matches within its bounded preview.
        // Hidden and typed hits wait for the engine bitmap.
        let confirmed = hit == Some(true);
        let searched = if confirmed { source } else { preview };
        let first = spans.len();
        spans.extend(find.text_match_ranges(searched, preview.len()));
        if confirmed && spans.len() == first {
            if preview.len() < source.len() {
                // The match lies past the preview: mark its ellipsis.
                spans.push(preview.len()..shown_len);
            } else if !text_matchable && shown_len > 0 {
                // Typed equality can match a differently formatted value.
                spans.push(0..shown_len);
            }
        }
    }

    /// The search field took focus.
    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.invalidate();
    }

    /// The search text changed. The step waits for typing to pause.
    pub fn set_text(&mut self, text: &str) -> FindWork {
        self.text = text.to_owned();
        self.restart(None, true)
    }

    /// Search another view, or none while a query is applied.
    pub fn set_view(&mut self, view: Option<Arc<OrderedView>>) -> FindWork {
        self.view = view;
        self.restart(None, false)
    }

    /// The visible columns changed. With `retain_match`, a match in a column still visible
    /// stays current: reordering does not move the search.
    pub fn set_columns(&mut self, columns: Vec<ColumnId>, retain_match: bool) -> FindWork {
        let hit = self
            .current()
            .filter(|hit| retain_match && columns.contains(&hit.column));
        self.viewport_columns.retain(|id| columns.contains(id));
        self.columns = columns;
        if self.view.is_none() {
            return FindWork::default();
        }
        self.restart(hit, false)
    }

    pub fn set_viewport(&mut self, rows: Range<u64>, columns: Vec<ColumnId>) -> FindWork {
        self.viewport_rows = rows;
        self.viewport_columns = columns;
        FindWork {
            highlight: self.next_highlight(),
            ..FindWork::default()
        }
    }

    /// Move a signed number of matches. A step requested while one runs follows it.
    pub fn step(&mut self, by: i64) -> FindWork {
        FindWork {
            step: self.advance(by, false),
            ..FindWork::default()
        }
    }

    pub fn step_finished(&mut self, completed: StepResult) -> FindWork {
        let Navigation::Running { queued } = self.navigation else {
            return FindWork::default();
        };
        if !self.is_current(&completed.find) {
            return FindWork::default();
        }
        self.navigation = Navigation::Idle;
        let mut work = FindWork::default();
        match completed.result {
            Ok((Some(hit), wrapped)) => {
                self.status = FindStatus::At { hit, wrapped };
                match &mut self.count {
                    Some(count) => count.movement += i128::from(completed.by),
                    None => work.count = Some(self.start_count(completed.find, hit)),
                }
                work.reveal = Some(hit);
            }
            Ok((None, _)) => self.status = FindStatus::NoMatches,
            Err(error) => {
                self.status = FindStatus::Failed {
                    error: error.into(),
                    at: completed.from,
                }
            }
        }
        if queued != 0 {
            work.step = self.advance(queued, false);
        }
        work
    }

    pub fn count_finished(&mut self, completed: CountResult) {
        let Some(count) = &mut self.count else {
            return;
        };
        if !Arc::ptr_eq(&count.cancel, &completed.cancel) {
            return;
        }
        match completed.result {
            Ok(Some(counts)) => count.start = Some(counts),
            Ok(None) => {}
            Err(error) => {
                self.count = None;
                self.fail(error.into());
            }
        }
    }

    pub fn highlight_finished(&mut self, completed: HighlightResult) -> FindWork {
        if !matches!(self.highlight_load, HighlightLoad::Reading)
            || !self.is_current(&completed.find)
        {
            return FindWork::default();
        }
        match completed.result {
            Ok(batch) => {
                self.highlight_load = HighlightLoad::Idle;
                self.highlights.store(batch);
                FindWork {
                    highlight: self.next_highlight(),
                    ..FindWork::default()
                }
            }
            Err(error) => {
                self.highlight_load = HighlightLoad::Failed;
                self.fail(error.into());
                FindWork::default()
            }
        }
    }

    fn is_current(&self, find: &Arc<Find>) -> bool {
        matches!(&self.search, Search::Ready(current) if Arc::ptr_eq(current, find))
    }

    fn fail(&mut self, error: Error) {
        self.status = FindStatus::Failed {
            error,
            at: self.current(),
        };
    }

    fn invalidate(&mut self) {
        self.search = Search::Unstarted;
        self.status = FindStatus::Idle;
        self.navigation = Navigation::Idle;
        self.count = None;
        self.highlights.0.clear();
        self.highlight_load = HighlightLoad::Idle;
    }

    /// Start over in a new scope, keeping `hit` current when it still belongs to it.
    fn restart(&mut self, hit: Option<CellMatch>, debounce: bool) -> FindWork {
        self.invalidate();
        let mut work = FindWork::default();
        if !self.open || self.view.is_none() {
            return work;
        }
        if let Some(hit) = hit {
            self.status = FindStatus::At {
                hit,
                wrapped: false,
            };
            work.count = self.find().map(|find| self.start_count(find, hit));
        } else {
            work.step = self.advance(1, debounce);
        }
        work.highlight = self.next_highlight();
        work
    }

    /// The search for the current text and scope, created on first use.
    fn find(&mut self) -> Option<Arc<Find>> {
        match &self.search {
            Search::Ready(find) => return Some(find.clone()),
            Search::Failed => return None,
            Search::Unstarted => {}
        }
        let view = self.view.as_ref()?;
        if self.text.trim().is_empty() || self.columns.is_empty() {
            return None;
        }
        match view.find(&self.text, &self.columns) {
            Ok(find) => {
                let find = Arc::new(find);
                self.search = Search::Ready(find.clone());
                Some(find)
            }
            Err(error) => {
                self.search = Search::Failed;
                self.fail(error);
                None
            }
        }
    }

    fn advance(&mut self, by: i64, debounce: bool) -> Option<StepJob> {
        if let Navigation::Running { queued } = &mut self.navigation {
            *queued = queued.saturating_add(by);
            return None;
        }
        let find = self.find()?;
        let from = self.current();
        self.status = FindStatus::Searching { from };
        self.navigation = Navigation::Running { queued: 0 };
        Some(StepJob {
            find,
            from,
            by,
            debounce,
        })
    }

    fn start_count(&mut self, find: Arc<Find>, hit: CellMatch) -> CountJob {
        let cancel = Arc::new(AtomicBool::new(false));
        self.count = Some(Count {
            start: None,
            movement: 0,
            cancel: cancel.clone(),
        });
        CountJob { find, hit, cancel }
    }

    /// The next run of uncached visible blocks, read in one batch up to Core's limit.
    fn next_highlight(&mut self) -> Option<HighlightJob> {
        let rows = self.viewport_rows.clone();
        if !self.open || !matches!(self.highlight_load, HighlightLoad::Idle) || rows.is_empty() {
            return None;
        }
        let find = self.find()?;
        let first = rows.start / HIGHLIGHT_BLOCK_ROWS;
        let end = rows.end.div_ceil(HIGHLIGHT_BLOCK_ROWS);
        let columns = &self.viewport_columns;
        let screen_blocks = columns.len() * (end - first) as usize;
        if self.highlights.0.len() > screen_blocks.saturating_mul(CACHED_SCREENS) {
            self.highlights
                .0
                .retain(|(id, block), _| columns.contains(id) && (first..end).contains(block));
        }
        let cached = |id: &ColumnId, block: u64| self.highlights.0.contains_key(&(*id, block));
        let block = (first..end).find(|&b| columns.iter().any(|id| !cached(id, b)))?;
        let missing: Vec<_> = columns
            .iter()
            .copied()
            .filter(|id| !cached(id, block))
            .collect();
        let max = end.min(block + u64::from(MAX_FIND_HIGHLIGHT_ROWS) / HIGHLIGHT_BLOCK_ROWS);
        let stop = (block + 1..max)
            .find(|&b| missing.iter().any(|id| cached(id, b)))
            .unwrap_or(max);
        self.highlight_load = HighlightLoad::Reading;
        Some(HighlightJob {
            find,
            columns: missing,
            start: block * HIGHLIGHT_BLOCK_ROWS,
            rows: ((stop - block) * HIGHLIGHT_BLOCK_ROWS) as u32,
        })
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
