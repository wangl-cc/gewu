use std::{
    collections::BTreeSet,
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::{Array, ArrayRef, Int64Array, UInt32Array},
    compute::{cast, concat_batches, lexsort_to_indices, take, SortColumn, SortOptions},
    datatypes::{DataType, SchemaRef},
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::{
    engine::{CancelToken, ViewEngine},
    format,
    numeric::Numeric,
    predicate::ColumnTest,
    query,
    scope::{RowScope, Selection},
    value_index::ValueIndex,
    Bound, ColumnInfo, ColumnStats, ColumnSummary, DataKind, DistinctResult, DistinctValue, Error,
    Export, ExportFormat, ExportOptions, FileInfo, Filter, Histogram, Result, RowBlock, ViewInfo,
    ViewState,
};

/// One open file. Safe to share between threads.
pub struct Document {
    info: FileInfo,
    source_path: PathBuf,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    n_rows: usize,
    indexes: Vec<OnceLock<Arc<ValueIndex>>>,
    views: ViewEngine,
}

impl Document {
    /// Open a Parquet file and decode it into memory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let source_path = path.canonicalize()?;
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
                kind: DataKind::of(f.data_type()),
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
            created_by: meta.file_metadata().created_by().map(str::to_string),
            metadata: meta
                .file_metadata()
                .key_value_metadata()
                .into_iter()
                .flatten()
                .filter(|kv| kv.key != "ARROW:schema")
                .map(|kv| (kv.key.clone(), kv.value.clone().unwrap_or_default()))
                .collect(),
        };

        let columns = batch.columns().to_vec();
        let indexes = (0..columns.len()).map(|_| OnceLock::new()).collect();

        Ok(Self {
            info,
            source_path,
            schema,
            columns,
            n_rows,
            indexes,
            views: ViewEngine::new(n_rows),
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
        self.views.current().info()
    }

    /// Capture a view and column order for export without copying its cells or
    /// row indices. The returned job remains valid when the current view changes.
    pub fn prepare_export(
        &self,
        version: u64,
        columns: &[u32],
        format: ExportFormat,
        options: ExportOptions,
    ) -> Result<Export> {
        let view = self.views.snapshot(version)?;
        if columns.is_empty() {
            return Err(Error::InvalidArgument(
                "select at least one column to export".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let arrays = columns
            .iter()
            .map(|&column| {
                if !seen.insert(column) {
                    return Err(Error::InvalidArgument("duplicate export column".into()));
                }
                self.column(column).cloned()
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = columns.iter().map(|&c| c as usize).collect::<Vec<_>>();
        let schema = Arc::new(self.schema.project(&indices)?);
        if format != ExportFormat::Parquet && !options.convert_nested_to_json {
            let nested = self.nested_export_columns(columns)?;
            if !nested.is_empty() {
                return Err(Error::Unsupported(format!(
                    "nested columns ({}) require JSON conversion for CSV/TSV; enable it or export as Parquet",
                    nested.join(", ")
                )));
            }
        }
        Ok(Export::new(
            self.source_path.clone(),
            schema,
            arrays,
            view.rows.clone(),
            format,
        ))
    }

    /// Names of selected columns that require conversion for CSV/TSV export.
    /// Inspect types only; this does not scan or copy any cells.
    pub fn nested_export_columns(&self, columns: &[u32]) -> Result<Vec<String>> {
        let mut nested = Vec::new();
        for &column in columns {
            if self.column(column)?.data_type().is_nested() {
                nested.push(self.schema.field(column as usize).name().clone());
            }
        }
        Ok(nested)
    }

    /// Cancel any `apply_view` still running.
    pub fn cancel_pending(&self) {
        self.views.cancel_pending();
    }

    /// Compute and switch to a new view. A newer call cancels this one, in
    /// which case it returns [`Error::Cancelled`] and the old view stays.
    pub fn apply_view(&self, state: &ViewState) -> Result<ViewInfo> {
        let token = self.views.begin();
        let mut rows = self.select(&state.filters, state.search.as_deref(), None, &token)?;
        token.check()?;
        if !state.sort.is_empty() {
            rows = self.sort(rows, state)?;
        }
        let search = state.search.clone().filter(|s| !s.trim().is_empty());
        self.views.commit(&token, rows, search)
    }

    /// Formatted cells for display rows `start..start+count` of view
    /// `version`. Rows past the end are left out.
    pub fn rows(&self, version: u64, start: u64, count: u32, columns: &[u32]) -> Result<RowBlock> {
        let view = self.views.snapshot(version)?;
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

        // Which cells match the search: the same tests the search filtered
        // with, run on just these rows. Nothing is kept afterwards.
        let mut matched = Vec::new();
        if let Some(text) = &view.search {
            let indices = UInt32Array::from(source_rows.to_vec());
            let token = CancelToken::never();
            let mut per_column = Vec::with_capacity(columns.len());
            for &c in columns {
                let column = &self.columns[c as usize];
                let mut hits = vec![false; source_rows.len()];
                if let Some(test) = query::search_test(column.data_type(), text)? {
                    let block = take(column.as_ref(), &indices, None)?;
                    test.mark(&block, &mut hits, &token)?;
                }
                per_column.push(hits);
            }
            matched = (0..source_rows.len())
                .flat_map(|r| per_column.iter().map(move |hits| hits[r]))
                .collect();
        }

        Ok(RowBlock {
            version,
            start,
            n_rows: source_rows.len() as u32,
            n_cols: columns.len() as u32,
            cells,
            source_rows: source_rows.iter().map(|&r| r as u64).collect(),
            matched,
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
        let rows = self.rows_in(RowScope::facet(filters, column))?;
        let present = value_counts(&idx, &rows);
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
        let rows = self.rows_in(RowScope::facet(filters, column))?;
        numeric.histogram(rows.iter(), bins)
    }

    /// Distribution over the displayed rows, including this column's own
    /// filters and the text search. The axis still spans the source column.
    pub fn view_histogram(&self, version: u64, column: u32, bins: u32) -> Result<Histogram> {
        let numeric = self.numeric(column)?;
        let view = self.views.snapshot(version)?;
        let rows = self.rows_in(RowScope::Rows(view.rows.clone()))?;
        numeric.histogram(rows.iter(), bins)
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
    /// Integer literals use exact Int/UInt bounds when representable. Fractional
    /// and float inputs use f64 without first narrowing to the column's type.
    /// Range preparation handles fractions and bounds outside the column's domain.
    pub fn parse_value(&self, column: u32, text: &str) -> Result<Bound> {
        self.numeric(column)?.parse_bound(text)
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

    /// Summary of one column over the current view's rows, with up to
    /// `top` most common values for non-numeric columns.
    pub fn column_summary(&self, column: u32, top: u32) -> Result<ColumnSummary> {
        let array = self.column(column)?.clone();
        let rows = self.rows_in(RowScope::Rows(self.views.current().rows.clone()))?;
        let mut s = ColumnSummary {
            rows: rows.len() as u64,
            null_count: 0,
            nan_count: 0,
            distinct_count: 0,
            min: None,
            max: None,
            mean: None,
            std_dev: None,
            top_values: Vec::new(),
        };
        match Numeric::new(&array) {
            Some(numeric) => numeric.summarize(rows.iter(), &mut s),
            None => {
                let idx = self.index(column)?;
                let mut values = value_counts(&idx, &rows);
                s.null_count = values
                    .iter()
                    .filter(|v| v.label.is_none())
                    .map(|v| v.count)
                    .sum();
                values.retain(|v| v.label.is_some());
                s.distinct_count = values.len() as u64;
                values.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
                values.truncate(top as usize);
                s.top_values = values;
            }
        }
        Ok(s)
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

    fn numeric(&self, c: u32) -> Result<Numeric<'_>> {
        let array = self.column(c)?;
        Numeric::new(array).ok_or_else(|| {
            Error::InvalidArgument(format!(
                "column {} ({}) is not numeric",
                self.info.columns[c as usize].name, self.info.columns[c as usize].type_name
            ))
        })
    }

    /// Resolve which rows a facet counts.
    fn rows_in(&self, scope: RowScope<'_>) -> Result<Selection> {
        Ok(match scope {
            RowScope::All => Selection::All(self.n_rows),
            RowScope::Rows(rows) => Selection::Rows(rows),
            RowScope::Facet { state, except } => {
                self.column(except)?;
                let rows = self.select(
                    &state.filters,
                    state.search.as_deref(),
                    Some(except),
                    &CancelToken::never(),
                )?;
                Selection::Rows(Arc::new(rows))
            }
        })
    }

    /// Source rows that pass all filters (except those on `skip`) and the
    /// search, in file order. Each filter marks a mask; masks are ANDed.
    fn select(
        &self,
        filters: &[Filter],
        search: Option<&str>,
        skip: Option<u32>,
        token: &CancelToken,
    ) -> Result<Vec<u32>> {
        let mut keep: Option<Vec<bool>> = None;
        for f in filters {
            if Some(f.column()) == skip {
                continue;
            }
            let test = self.filter_test(f)?;
            let mut mask = vec![false; self.n_rows];
            test.mark(self.column(f.column())?, &mut mask, token)?;
            and_into(&mut keep, mask);
        }

        if let Some(text) = search.filter(|s| !s.trim().is_empty()) {
            // Any column may match: OR the columns into one mask.
            let mut mask = vec![false; self.n_rows];
            for column in &self.columns {
                if let Some(test) = query::search_test(column.data_type(), text)? {
                    test.mark(column, &mut mask, token)?;
                }
            }
            and_into(&mut keep, mask);
        }

        token.check()?;
        Ok(match keep {
            None => (0..self.n_rows as u32).collect(),
            Some(keep) => keep
                .iter()
                .enumerate()
                .filter(|&(_, &k)| k)
                .map(|(r, _)| r as u32)
                .collect(),
        })
    }

    /// The test a filter means, after checking it fits the column.
    fn filter_test(&self, filter: &Filter) -> Result<ColumnTest> {
        let column = filter.column();
        self.column(column)?;
        let info = &self.info.columns[column as usize];
        Ok(match filter {
            Filter::InValues { keys, .. } => {
                let index = self.index(column)?;
                let mut allowed = vec![false; index.len()];
                for &k in keys {
                    if let Some(slot) = allowed.get_mut(k as usize) {
                        *slot = true;
                    }
                }
                ColumnTest::Codes { index, allowed }
            }
            Filter::IsNull { negate, .. } => ColumnTest::Null { negate: *negate },
            Filter::Contains {
                text,
                case_sensitive,
                ..
            } => {
                if info.kind.is_ordered() || info.kind == DataKind::Boolean {
                    return Err(Error::InvalidArgument(format!(
                        "\"contains\" is for text; column {} holds {:?} values, filter it by range or value",
                        info.name, info.kind
                    )));
                }
                ColumnTest::Text {
                    text: text.clone(),
                    case_sensitive: *case_sensitive,
                }
            }
            Filter::Range { min, max, .. } => {
                if !info.kind.is_ordered() {
                    return Err(Error::InvalidArgument(format!(
                        "column {} ({}) is not numeric",
                        info.name, info.type_name
                    )));
                }
                ColumnTest::Range {
                    min: *min,
                    max: *max,
                }
            }
        })
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

/// Count each distinct value over `rows`, in value order, leaving out
/// values with no rows.
fn value_counts(idx: &ValueIndex, rows: &Selection) -> Vec<DistinctValue> {
    let mut counts = vec![0u64; idx.len()];
    match rows {
        Selection::All(_) => idx.codes.iter().for_each(|&c| counts[c as usize] += 1),
        Selection::Rows(rows) => rows
            .iter()
            .for_each(|&r| counts[idx.codes[r as usize] as usize] += 1),
    }
    counts
        .into_iter()
        .enumerate()
        .filter(|&(_, n)| n > 0)
        .map(|(code, count)| DistinctValue {
            key: code as u64,
            label: idx.labels[code].clone(),
            count,
        })
        .collect()
}

/// AND `mask` into `keep` (which starts as "everything").
fn and_into(keep: &mut Option<Vec<bool>>, mask: Vec<bool>) {
    match keep {
        None => *keep = Some(mask),
        Some(keep) => keep.iter_mut().zip(mask).for_each(|(k, m)| *k &= m),
    }
}
