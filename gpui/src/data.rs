//! The table's data, read straight from gewu-core: no FFI, no packing.
//!
//! Display text is formatted by Core per (column, 64-row block) and cached as
//! a native column segment. Only visible cells acquire owned GPUI text; it is reused
//! on subsequent frames. Installing a new displayed view invalidates the blocks.

use std::{cell::OnceCell, collections::HashMap, ops::Range, sync::Arc};

use gewu_app::{QueryCompletion, QueryJob, QueryResult, QuerySession};
use gewu_core::{
    ColumnId, Dataset, ExportOptions, FileType, FormattedColumnSegment, OrderedView, ValueCategory,
};
use gpui_kit::SharedString;

pub const BLOCK_ROWS: u64 = 64;
/// Limit the encoded TSV before handing it to the pasteboard.
pub const COPY_BYTE_LIMIT: usize = 64 * 1024 * 1024;
pub(crate) const GRID_PREVIEW_BYTES: usize = 1024;

/// The grid shapes a single line. Keep the complete engine text available for details and copy.
pub(crate) fn preview(text: &str) -> SharedString {
    preview_with_limit(text, GRID_PREVIEW_BYTES)
}

pub(crate) fn preview_with_limit(text: &str, max_bytes: usize) -> SharedString {
    let prefix = single_line_prefix(text, max_bytes);
    if prefix.len() == text.len() {
        SharedString::new(text)
    } else {
        format!("{prefix}…").into()
    }
}

pub(crate) fn single_line_prefix(text: &str, max_bytes: usize) -> &str {
    let limit = text.floor_char_boundary(text.len().min(max_bytes));
    let end = text[..limit]
        .find(['\r', '\n', '\u{85}', '\u{2028}', '\u{2029}'])
        .unwrap_or(limit);
    &text[..end]
}

const CACHE_BYTES: usize = 64 * 1024 * 1024;

enum BlockText {
    Ready {
        segment: FormattedColumnSegment,
        visible: Vec<OnceCell<SharedString>>,
        originals: Vec<Option<SharedString>>,
    },
    Failed(SharedString),
}

struct Block {
    text: BlockText,
    tick: u64,
    budget: usize,
}

impl Block {
    fn new(text: gewu_core::Result<FormattedColumnSegment>, tick: u64) -> Self {
        match text {
            Ok(segment) => {
                let len = segment.row_count();
                let visible = (0..len).map(|_| OnceCell::new()).collect();
                // Reserve the native text and one owned copy, including a possible preview
                // ellipsis.
                let budget = 2 * segment.text_bytes()
                    + len as usize
                        * (size_of::<OnceCell<SharedString>>() + size_of::<u32>() + 1 + "…".len());
                Self {
                    text: BlockText::Ready {
                        segment,
                        visible,
                        originals: Vec::new(),
                    },
                    tick,
                    budget,
                }
            }
            Err(error) => {
                let message: SharedString = crate::errors::message(&error).into();
                Self {
                    budget: message.len(),
                    text: BlockText::Failed(message),
                    tick,
                }
            }
        }
    }

    fn preview(&self, row: u32) -> Option<Option<SharedString>> {
        match &self.text {
            BlockText::Ready {
                segment, visible, ..
            } => {
                let rendered = visible.get(row as usize)?;
                Some(
                    segment
                        .cell(row)
                        .map(|text| rendered.get_or_init(|| preview(text)).clone()),
                )
            }
            BlockText::Failed(_) => None,
        }
    }

    fn full_text(&mut self, row: u32) -> Option<Option<SharedString>> {
        let BlockText::Ready {
            segment,
            visible,
            originals,
        } = &mut self.text
        else {
            return None;
        };
        let index = row as usize;
        let rendered = visible.get(index)?;
        Some(segment.cell(row).map(|text| {
            let shown = rendered.get_or_init(|| preview(text));
            if shown.as_ref() == text {
                return shown.clone();
            }
            if originals.is_empty() {
                originals.resize(visible.len(), None);
                self.budget += originals.capacity() * size_of::<Option<SharedString>>();
            }
            originals[index]
                .get_or_insert_with(|| {
                    // The full copy was reserved at block creation. Also retain the truncated
                    // preview.
                    self.budget += shown.len();
                    SharedString::new(text)
                })
                .clone()
        }))
    }
}

pub use gewu_app::{AppliedView, TableQuery};

pub struct TableData {
    queries: QuerySession,
    pub columns: crate::columns::TableColumns,
    cache: HashMap<(ColumnId, u64), Block>,
    cache_bytes: usize,
    tick: u64,
}

impl TableData {
    #[cfg(test)]
    pub fn open(path: &str) -> gewu_core::Result<Self> {
        Ok(Self::from_dataset(Arc::new(Dataset::open(path)?)))
    }

    pub fn from_dataset(dataset: Arc<Dataset>) -> Self {
        let columns = crate::columns::TableColumns::new(dataset.column_infos());
        Self {
            queries: QuerySession::new(AppliedView::new(dataset)),
            columns,
            cache: HashMap::new(),
            cache_bytes: 0,
            tick: 0,
        }
    }

    pub(crate) fn display_cell(
        &self,
        row: gewu_core::ViewRow,
        id: ColumnId,
    ) -> Option<crate::selection::Cell> {
        let column = self.columns.position(id)?;
        (row.0 < self.row_count()).then_some(crate::selection::Cell {
            row: row.0,
            column: column.index(),
        })
    }

    pub fn source_row_count(&self) -> usize {
        self.queries.displayed().dataset().n_rows()
    }

    pub fn row_count(&self) -> u64 {
        self.view().row_count()
    }

    pub fn is_number(&self, column: usize) -> bool {
        self.columns[column].value_category == ValueCategory::Number
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
    }

    pub fn snapshot(&self) -> AppliedView {
        self.queries.displayed().clone()
    }

    pub fn query(&self) -> &Arc<TableQuery> {
        self.queries.displayed().query()
    }

    pub fn view(&self) -> &Arc<OrderedView> {
        self.queries.displayed().view()
    }

    pub(crate) fn queries(&self) -> &QuerySession {
        &self.queries
    }

    pub(crate) fn edit_query(&mut self, edit: impl FnOnce(&mut TableQuery)) -> Option<QueryJob> {
        self.queries.edit(edit)
    }

    pub(crate) fn finish_query(&mut self, completed: QueryResult) -> QueryCompletion {
        let change = self.queries.finish(completed);
        if matches!(change, QueryCompletion::Installed) {
            self.clear_cache();
        }
        change
    }

    /// Format missing blocks and keep the visible working set resident.
    pub fn prefetch(
        &mut self,
        columns: impl IntoIterator<Item = usize>,
        rows: Range<u64>,
    ) -> Option<SharedString> {
        self.tick += 1;
        if rows.is_empty() {
            return None;
        }
        let mut error = None;
        let blocks = rows.start / BLOCK_ROWS..=(rows.end - 1) / BLOCK_ROWS;
        for column in columns {
            for block in blocks.clone() {
                let key = (self.columns[column].id, block);
                let entry = self.cache.entry(key).or_insert_with(|| {
                    let cells = Block::new(
                        self.queries.displayed().view().format_column(
                            key.0,
                            block * BLOCK_ROWS,
                            BLOCK_ROWS as u32,
                        ),
                        self.tick,
                    );
                    self.cache_bytes = self.cache_bytes.saturating_add(cells.budget);
                    cells
                });
                entry.tick = self.tick;
                if error.is_none()
                    && let BlockText::Failed(message) = &entry.text
                {
                    error = Some(message.clone());
                }
            }
        }
        if self.cache_bytes > CACHE_BYTES {
            let mut old: Vec<_> = self
                .cache
                .iter()
                .filter(|(_, v)| v.tick != self.tick)
                .map(|(&key, v)| (v.tick, key))
                .collect();
            old.sort_unstable();
            for (_, key) in old {
                if self.cache_bytes <= CACHE_BYTES / 2 {
                    break;
                }
                if let Some(block) = self.cache.remove(&key) {
                    self.cache_bytes -= block.budget;
                }
            }
        }
        error
    }

    /// Cached text of a cell: `Some(None)` is null; unfetched or failed blocks have no value.
    pub fn cell(&mut self, column: usize, row: u64) -> Option<Option<SharedString>> {
        let block = self
            .cache
            .get_mut(&(self.columns.get(column)?.id, row / BLOCK_ROWS))?;
        let before = block.budget;
        let text = block.full_text((row % BLOCK_ROWS) as u32);
        self.cache_bytes += block.budget - before;
        text
    }

    pub(crate) fn cached_cell_text(&self, column: usize, row: u64) -> Option<Option<&str>> {
        let block = self
            .cache
            .get(&(self.columns.get(column)?.id, row / BLOCK_ROWS))?;
        let relative = (row % BLOCK_ROWS) as u32;
        match &block.text {
            BlockText::Ready { segment, .. } if relative < segment.row_count() => {
                Some(segment.cell(relative))
            }
            _ => None,
        }
    }

    pub fn cell_preview(&self, column: usize, row: u64) -> Option<Option<SharedString>> {
        self.cache
            .get(&(self.columns.get(column)?.id, row / BLOCK_ROWS))?
            .preview((row % BLOCK_ROWS) as u32)
    }

    pub fn column_ids(&self, columns: Range<usize>) -> Vec<ColumnId> {
        columns.map(|index| self.columns[index].id).collect()
    }
}

/// TSV of display rows x columns through Core's export path, as the Mac app copies.
pub fn copy_tsv(
    view: &OrderedView,
    rows: Range<u64>,
    columns: &[ColumnId],
    include_header: bool,
) -> gewu_core::Result<String> {
    let export = view.export_rows(&[rows], columns, FileType::Tsv)?;
    let options = ExportOptions {
        nested_as_text: true,
        include_header,
        ..ExportOptions::default()
    };
    let bytes = export.write_to_memory(&options, COPY_BYTE_LIMIT)?;
    String::from_utf8(bytes).map_err(|error| {
        gewu_core::InternalError::Invariant(format!("Core exported invalid UTF-8 TSV: {error}"))
            .into()
    })
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
