//! The table's data, read straight from gewu-core: no FFI, no packing.
//!
//! Display text is formatted by Core per (column, 64-row block) and cached as
//! native column segment. Only visible cells acquire owned GPUI text; it is reused
//! on subsequent frames. Installing a new displayed view invalidates the blocks.

use std::{
    cell::OnceCell,
    collections::HashMap,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use gewu_core::{
    ColumnFilter, ColumnId, ColumnInfo, ColumnSort, Dataset, ExportOptions, FileType, FilteredView,
    FormattedColumnSegment, OrderedView, SourceRows, ValueCategory,
};

pub const BLOCK_ROWS: u64 = 64;
/// Copy limit, the same as the Mac app's `CopyModel.byteLimit`.
pub const COPY_BYTE_LIMIT: usize = 64 * 1024 * 1024;
/// Cached blocks before the cache is trimmed to the ones last used.
const CACHE_BLOCKS: usize = 4096;

use gpui_kit::SharedString;

const CACHE_BYTES: usize = 64 * 1024 * 1024;

enum BlockText {
    Ready {
        segment: FormattedColumnSegment,
        visible: Vec<OnceCell<SharedString>>,
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
                // Reserve both the native segment and a possible rendered copy of its bytes.
                let bytes: usize = (0..len)
                    .filter_map(|row| segment.cell(row))
                    .map(str::len)
                    .sum();
                let budget = 2 * bytes
                    + len as usize * (size_of::<OnceCell<SharedString>>() + size_of::<u32>() + 1);
                Self {
                    text: BlockText::Ready { segment, visible },
                    tick,
                    budget,
                }
            }
            Err(error) => {
                let message: SharedString = format!("⚠ {error}").into();
                Self {
                    budget: message.len(),
                    text: BlockText::Failed(message),
                    tick,
                }
            }
        }
    }

    fn cell(&self, row: u32) -> Option<Option<SharedString>> {
        match &self.text {
            BlockText::Ready { segment, visible } => {
                let rendered = visible.get(row as usize)?;
                Some(
                    segment
                        .cell(row)
                        .map(|text| rendered.get_or_init(|| SharedString::new(text)).clone()),
                )
            }
            BlockText::Failed(message) => Some(Some(message.clone())),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TableQuery {
    pub filters: Vec<ColumnFilter>,
    pub sorts: Vec<ColumnSort>,
}

/// A query and the immutable views computed from it.
#[derive(Clone)]
pub struct AppliedView {
    dataset: Arc<Dataset>,
    query: Arc<TableQuery>,
    filtered: Arc<FilteredView>,
    view: Arc<OrderedView>,
}

impl AppliedView {
    pub fn applying(&self, query: Arc<TableQuery>) -> gewu_core::Result<Self> {
        let filtered = if query.filters == self.query.filters {
            self.filtered.clone()
        } else {
            self.dataset.filter(&query.filters)?
        };
        let view = filtered.sort(&query.sorts)?;
        Ok(Self {
            dataset: self.dataset.clone(),
            query,
            filtered,
            view,
        })
    }
}

pub struct TableData {
    applied: AppliedView,
    pub columns: Vec<ColumnInfo>,
    cache: HashMap<(u32, u64), Block>,
    cache_bytes: usize,
    tick: u64,
}

impl TableData {
    pub fn open(path: &str) -> gewu_core::Result<Self> {
        let dataset = Arc::new(Dataset::open(path)?);
        let filtered = dataset.unfiltered();
        let view = filtered.unsorted();
        let columns = dataset.info().columns;
        Ok(Self {
            applied: AppliedView {
                dataset,
                filtered,
                view,
                query: Arc::default(),
            },
            columns,
            cache: HashMap::new(),
            cache_bytes: 0,
            tick: 0,
        })
    }

    pub fn row_count(&self) -> u64 {
        self.applied.view.row_count()
    }

    pub fn is_number(&self, column: usize) -> bool {
        self.columns[column].value_category == ValueCategory::Number
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
    }

    pub fn snapshot(&self) -> AppliedView {
        self.applied.clone()
    }

    pub fn query(&self) -> &Arc<TableQuery> {
        &self.applied.query
    }

    pub fn view(&self) -> &Arc<OrderedView> {
        &self.applied.view
    }

    pub fn install(&mut self, applied: AppliedView) {
        self.applied = applied;
        self.clear_cache();
    }

    /// Format every missing block of `columns` x `rows`; returns the time spent in Core.
    pub fn prefetch(&mut self, columns: &[usize], rows: Range<u64>) -> Duration {
        self.tick += 1;
        let mut spent = Duration::ZERO;
        if rows.is_empty() {
            return spent;
        }
        let blocks = rows.start / BLOCK_ROWS..=(rows.end - 1) / BLOCK_ROWS;
        for &column in columns {
            for block in blocks.clone() {
                let key = (column as u32, block);
                if let Some(entry) = self.cache.get_mut(&key) {
                    entry.tick = self.tick;
                    continue;
                }
                let t = Instant::now();
                let cells = Block::new(
                    self.applied.view.format_column(
                        self.columns[column].id,
                        block * BLOCK_ROWS,
                        BLOCK_ROWS as u32,
                    ),
                    self.tick,
                );
                spent += t.elapsed();
                self.cache_bytes = self.cache_bytes.saturating_add(cells.budget);
                self.cache.insert(key, cells);
            }
        }
        if self.cache.len() > CACHE_BLOCKS || self.cache_bytes > CACHE_BYTES {
            let mut old: Vec<_> = self
                .cache
                .iter()
                .filter(|(_, v)| v.tick != self.tick)
                .map(|(&key, v)| (v.tick, key))
                .collect();
            old.sort_unstable();
            for (_, key) in old {
                if self.cache.len() <= CACHE_BLOCKS / 2 && self.cache_bytes <= CACHE_BYTES / 2 {
                    break;
                }
                if let Some(block) = self.cache.remove(&key) {
                    self.cache_bytes -= block.budget;
                }
            }
        }
        spent
    }

    /// Cached text of a cell: `Some(None)` is null, `None` means not fetched.
    pub fn cell(&self, column: usize, row: u64) -> Option<Option<SharedString>> {
        self.cache
            .get(&(column as u32, row / BLOCK_ROWS))?
            .cell((row % BLOCK_ROWS) as u32)
    }

    /// The source (file) row of a display row.
    pub fn source_row(&self, row: u64) -> u64 {
        match self.applied.view.row_ids() {
            SourceRows::All(_) => row,
            SourceRows::Selected(rows) => rows[row as usize] as u64,
        }
    }

    pub fn column_ids(&self, columns: Range<usize>) -> Vec<ColumnId> {
        self.columns[columns].iter().map(|c| c.id).collect()
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
mod tests {
    use super::*;

    fn sample() -> TableData {
        TableData::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workflow.csv")).unwrap()
    }

    #[test]
    fn cached_blocks_are_replaced_with_the_sorted_view() {
        let mut data = sample();
        data.prefetch(&[0], 0..3);
        assert_eq!(data.cell(0, 1).unwrap().as_deref(), Some("drop"));
        assert!(data.cell(0, 200).is_none());
        assert_eq!(data.prefetch(&[0], 1..2), Duration::ZERO);
        let sort = ColumnSort {
            column: data.columns[1].id,
            descending: true,
        };
        let sorted = data
            .applied
            .applying(Arc::new(TableQuery {
                sorts: vec![sort],
                ..TableQuery::default()
            }))
            .unwrap();
        assert!(Arc::ptr_eq(&data.applied.filtered, &sorted.filtered));
        data.install(sorted);
        assert_eq!(data.query().sorts, [sort]);
        assert!(data.cell(0, 1).is_none());
        data.prefetch(&[0], 1..2);
        assert_eq!(data.cell(0, 0).unwrap().as_deref(), Some("drop"));
    }

    #[test]
    fn copy_uses_core_tsv_export() {
        let data = sample();
        let ids = data.column_ids(0..2);
        let text = copy_tsv(data.view(), 1..3, &ids, true).unwrap();
        assert_eq!(text.lines().collect::<Vec<_>>(), [
            "group\tscore",
            "drop\t99",
            "keep\t5"
        ]);
    }
}
