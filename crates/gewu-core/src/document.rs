use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use arrow::array::{Array, ArrayRef, Int64Array, StringArray, UInt32Array};
use arrow::compute::{cast, concat_batches, lexsort_to_indices, take, SortColumn, SortOptions};
use arrow::datatypes::{DataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::numeric::Numeric;
use crate::search;
use crate::value_index::ValueIndex;
use crate::{
    format, Bound, ColumnInfo, ColumnStats, DistinctResult, DistinctValue, Error, FileInfo, Filter,
    Histogram, Result, RowBlock, ViewInfo, ViewState,
};

/// How many rows to process between cancellation checks.
const CHUNK: usize = 64 * 1024;

/// Tells a long-running operation that a newer one has replaced it.
#[derive(Clone)]
pub struct CancelToken {
    counter: Arc<AtomicU64>,
    mine: u64,
}

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.counter.load(Ordering::SeqCst) != self.mine
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

struct View {
    version: u64,
    /// Source row numbers in display order.
    rows: Arc<Vec<u32>>,
}

/// One open file. Safe to share between threads.
pub struct Document {
    info: FileInfo,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    n_rows: usize,
    indexes: Vec<OnceLock<Arc<ValueIndex>>>,
    numerics: Vec<OnceLock<Option<Arc<Numeric>>>>,
    current: Mutex<Arc<View>>,
    generation: Arc<AtomicU64>,
    next_version: AtomicU64,
}

impl Document {
    /// Open a Parquet file and decode it into memory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let file_size = file.metadata()?.len();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;

        let meta = builder.metadata().clone();
        let schema = builder.schema().clone();
        let compression: BTreeSet<String> = meta
            .row_groups()
            .iter()
            .flat_map(|rg| rg.columns().iter())
            .map(|c| {
                let s = c.compression().to_string();
                s.split('(').next().unwrap_or(&s).to_string()
            })
            .collect();

        let reader = builder.with_batch_size(64 * 1024).build()?;
        let batches = reader.collect::<Result<Vec<_>, _>>()?;
        let batch = concat_batches(&schema, &batches)?;
        drop(batches);

        let n_rows = batch.num_rows();
        if n_rows > u32::MAX as usize {
            return Err(Error::Unsupported(format!(
                "{n_rows} rows is more than the supported {}",
                u32::MAX
            )));
        }

        let columns_info = schema
            .fields()
            .iter()
            .map(|f| ColumnInfo {
                name: f.name().clone(),
                type_name: f.data_type().to_string(),
                nullable: f.is_nullable(),
            })
            .collect();

        let info = FileInfo {
            path: path.display().to_string(),
            row_count: n_rows as u64,
            columns: columns_info,
            row_groups: meta.num_row_groups() as u32,
            file_size,
            compression: compression.into_iter().collect(),
        };

        let columns = batch.columns().to_vec();
        let indexes = (0..columns.len()).map(|_| OnceLock::new()).collect();
        let numerics = (0..columns.len()).map(|_| OnceLock::new()).collect();
        let all_rows = Arc::new((0..n_rows as u32).collect());

        Ok(Self {
            info,
            schema,
            columns,
            n_rows,
            indexes,
            numerics,
            current: Mutex::new(Arc::new(View {
                version: 0,
                rows: all_rows,
            })),
            generation: Arc::new(AtomicU64::new(0)),
            next_version: AtomicU64::new(1),
        })
    }

    pub fn info(&self) -> &FileInfo {
        &self.info
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Index of the column with this name.
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.info
            .columns
            .iter()
            .position(|c| c.name == name)
            .map(|i| i as u32)
    }

    pub fn current_view(&self) -> ViewInfo {
        let v = self.current.lock().unwrap();
        ViewInfo {
            version: v.version,
            row_count: v.rows.len() as u64,
        }
    }

    /// Cancel any `apply_view` still running.
    pub fn cancel_pending(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Compute and switch to a new view. A newer call cancels this one, in
    /// which case it returns [`Error::Cancelled`] and the old view stays.
    pub fn apply_view(&self, state: &ViewState) -> Result<ViewInfo> {
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let token = CancelToken {
            counter: self.generation.clone(),
            mine,
        };

        let mut rows = self.select(&state.filters, state.search.as_deref(), None, &token)?;
        token.check()?;
        if !state.sort.is_empty() {
            rows = self.sort(rows, state)?;
        }

        // Check and swap under the lock so a cancelled run never commits.
        let mut current = self.current.lock().unwrap();
        token.check()?;
        let version = self.next_version.fetch_add(1, Ordering::SeqCst);
        let info = ViewInfo {
            version,
            row_count: rows.len() as u64,
        };
        *current = Arc::new(View {
            version,
            rows: Arc::new(rows),
        });
        Ok(info)
    }

    /// Formatted cells for display rows `start..start+count` of view
    /// `version`. Rows past the end are left out.
    pub fn rows(&self, version: u64, start: u64, count: u32, columns: &[u32]) -> Result<RowBlock> {
        let view = self.current.lock().unwrap().clone();
        if view.version != version {
            return Err(Error::StaleVersion {
                requested: version,
                current: view.version,
            });
        }
        for &c in columns {
            self.column(c)?;
        }

        let len = view.rows.len() as u64;
        let start = start.min(len);
        let end = start.saturating_add(count as u64).min(len);
        let source_rows = &view.rows[start as usize..end as usize];

        let arrays: Vec<&dyn Array> = columns
            .iter()
            .map(|&c| self.columns[c as usize].as_ref())
            .collect();
        let formatters = arrays
            .iter()
            .map(|a| format::formatter(*a))
            .collect::<Result<Vec<_>>>()?;

        let mut cells = Vec::with_capacity(source_rows.len() * columns.len());
        for &r in source_rows {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(*a, f, r as usize));
            }
        }

        Ok(RowBlock {
            version,
            start,
            n_rows: source_rows.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: source_rows.iter().map(|&r| r as u64).collect(),
        })
    }

    /// Distinct values of a column with their counts, for the checkbox filter.
    ///
    /// With `filters`, counts only rows that pass the filters on *other*
    /// columns (like Excel), and leaves out values with no such rows.
    pub fn distinct_values(
        &self,
        column: u32,
        filters: Option<&ViewState>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let idx = self.index(column)?;
        let mut counts = vec![0u64; idx.len()];
        match filters {
            Some(state) => {
                let token = self.detached_token();
                let rows = self.select(
                    &state.filters,
                    state.search.as_deref(),
                    Some(column),
                    &token,
                )?;
                for r in rows {
                    counts[idx.codes[r as usize] as usize] += 1;
                }
            }
            None => {
                for &c in &idx.codes {
                    counts[c as usize] += 1;
                }
            }
        }

        let present: Vec<DistinctValue> = counts
            .iter()
            .enumerate()
            .filter(|(_, &n)| n > 0)
            .map(|(code, &n)| DistinctValue {
                key: code as u64,
                label: idx.labels[code].clone(),
                count: n,
            })
            .collect();
        let total = present.len() as u64;
        let truncated = present.len() > limit as usize;
        let values = present.into_iter().take(limit as usize).collect();
        Ok(DistinctResult {
            values,
            total,
            truncated,
        })
    }

    /// Distribution of a numeric, date or time column in `bins` bins, for
    /// the range filter. With `filters`, counts only rows that pass the
    /// filters on *other* columns, like [`Self::distinct_values`].
    pub fn histogram(
        &self,
        column: u32,
        filters: Option<&ViewState>,
        bins: u32,
    ) -> Result<Histogram> {
        let numeric = self.numeric(column)?;
        match filters {
            Some(state) => {
                let token = self.detached_token();
                let rows = self.select(
                    &state.filters,
                    state.search.as_deref(),
                    Some(column),
                    &token,
                )?;
                numeric.histogram(rows.into_iter().map(|r| r as usize), bins)
            }
            None => numeric.histogram(0..self.n_rows, bins),
        }
    }

    /// Display text for a raw value of a date, time, timestamp or duration
    /// column (its integer physical value, as used by histograms and range
    /// bounds), formatted exactly like the table's cells. Also works for
    /// integer columns.
    pub fn format_value(&self, column: u32, value: i64) -> Result<String> {
        let data_type = self.column(column)?.data_type().clone();
        let raw: ArrayRef = Arc::new(Int64Array::from(vec![value]));
        let typed = match data_type {
            // Arrow casts these from 32-bit integers only.
            DataType::Date32 | DataType::Time32(_) => {
                cast(&cast(&raw, &DataType::Int32)?, &data_type)?
            }
            _ => cast(&raw, &data_type)?,
        };
        let f = format::formatter(typed.as_ref())?;
        Ok(f.value(0).to_string())
    }

    /// Read text typed by the user (a number, or a date/time in the
    /// column's own format such as `2024-03-01` or `2024-03-01T12:00:00`)
    /// as a range bound for a numeric, date or time column.
    pub fn parse_value(&self, column: u32, text: &str) -> Result<Bound> {
        let data_type = self.column(column)?.data_type().clone();
        let input: ArrayRef = Arc::new(StringArray::from(vec![text.trim()]));
        let typed = cast(&input, &data_type)?;
        let bound = Numeric::build(&typed)?.and_then(|n| n.bound(0));
        bound.ok_or_else(|| Error::InvalidArgument(format!("cannot read {text:?} as {data_type}")))
    }

    /// Keys for the given displayed labels (`None` = null). Unknown labels
    /// are ignored. Used by the CLI and MCP, which filter by value text.
    pub fn keys_for_labels(&self, column: u32, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let idx = self.index(column)?;
        Ok(idx
            .labels
            .iter()
            .enumerate()
            .filter(|(_, l)| labels.contains(l))
            .map(|(k, _)| k as u64)
            .collect())
    }

    pub fn column_stats(&self, column: u32) -> Result<ColumnStats> {
        let array = self.column(column)?;
        let idx = self.index(column)?;
        let non_null: Vec<&String> = idx.labels.iter().flatten().collect();
        Ok(ColumnStats {
            null_count: array.null_count() as u64,
            distinct_count: non_null.len() as u64,
            min: non_null.first().map(|s| s.to_string()),
            max: non_null.last().map(|s| s.to_string()),
        })
    }

    // ---- internals ----

    fn column(&self, c: u32) -> Result<&ArrayRef> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }

    fn index(&self, c: u32) -> Result<Arc<ValueIndex>> {
        let array = self.column(c)?;
        let cell = &self.indexes[c as usize];
        if let Some(idx) = cell.get() {
            return Ok(idx.clone());
        }
        let built = Arc::new(ValueIndex::build(array)?);
        Ok(cell.get_or_init(|| built).clone())
    }

    fn numeric(&self, c: u32) -> Result<Arc<Numeric>> {
        let array = self.column(c)?;
        let cell = &self.numerics[c as usize];
        let n = match cell.get() {
            Some(n) => n.clone(),
            None => {
                let built = Numeric::build(array)?.map(Arc::new);
                cell.get_or_init(|| built).clone()
            }
        };
        n.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "column {} ({}) is not numeric",
                self.info.columns[c as usize].name, self.info.columns[c as usize].type_name
            ))
        })
    }

    /// A token that is never cancelled, for short read-only work.
    fn detached_token(&self) -> CancelToken {
        let counter = Arc::new(AtomicU64::new(0));
        CancelToken { counter, mine: 0 }
    }

    /// Source rows that pass all filters (except those on `skip`).
    fn select(
        &self,
        filters: &[Filter],
        search: Option<&str>,
        skip: Option<u32>,
        token: &CancelToken,
    ) -> Result<Vec<u32>> {
        // Turn each filter into a per-row test.
        let mut tests: Vec<Box<dyn Fn(usize) -> bool + Send + Sync>> = Vec::new();
        for f in filters {
            if Some(f.column()) == skip {
                continue;
            }
            match f {
                Filter::InValues { column, keys } => {
                    let idx = self.index(*column)?;
                    let mut allowed = vec![false; idx.len()];
                    for &k in keys {
                        if let Some(slot) = allowed.get_mut(k as usize) {
                            *slot = true;
                        }
                    }
                    tests.push(Box::new(move |r| allowed[idx.codes[r] as usize]));
                }
                Filter::IsNull { column, negate } => {
                    let array = self.column(*column)?.clone();
                    let negate = *negate;
                    tests.push(Box::new(move |r| array.is_null(r) != negate));
                }
                Filter::Contains {
                    column,
                    text,
                    case_sensitive,
                } => {
                    let mut hits = vec![false; self.n_rows];
                    search::mark_matches(
                        self.column(*column)?,
                        text,
                        *case_sensitive,
                        &mut hits,
                        token,
                    )?;
                    tests.push(Box::new(move |r| hits[r]));
                }
                Filter::Range { column, min, max } => {
                    tests.push(self.numeric(*column)?.range_test(*min, *max));
                }
            }
            token.check()?;
        }

        if let Some(text) = search.filter(|s| !s.is_empty()) {
            // One bitmap for all columns; nothing is cached afterwards.
            let mut hits = vec![false; self.n_rows];
            for column in &self.columns {
                search::mark_matches(column, text, false, &mut hits, token)?;
            }
            tests.push(Box::new(move |r| hits[r]));
        }

        let mut rows = Vec::new();
        for chunk_start in (0..self.n_rows).step_by(CHUNK) {
            token.check()?;
            let end = (chunk_start + CHUNK).min(self.n_rows);
            rows.extend(
                (chunk_start..end)
                    .filter(|&r| tests.iter().all(|t| t(r)))
                    .map(|r| r as u32),
            );
        }
        Ok(rows)
    }

    fn sort(&self, rows: Vec<u32>, state: &ViewState) -> Result<Vec<u32>> {
        let selected = UInt32Array::from(rows);
        let sort_columns = state
            .sort
            .iter()
            .map(|k| {
                let array = self.column(k.column)?;
                Ok(SortColumn {
                    values: take(array.as_ref(), &selected, None)?,
                    options: Some(SortOptions {
                        descending: k.descending,
                        // Nulls always at the bottom, like spreadsheets.
                        nulls_first: false,
                    }),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let order = lexsort_to_indices(&sort_columns, None)?;
        let selected = selected.values();
        Ok(order
            .values()
            .iter()
            .map(|&i| selected[i as usize])
            .collect())
    }
}
