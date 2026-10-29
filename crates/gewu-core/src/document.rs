use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::{make_comparator, Array, ArrayRef, Int64Array, UInt32Array},
    compute::{cast, lexsort_to_indices, take, SortColumn, SortOptions},
    datatypes::{DataType, SchemaRef},
};

use crate::{
    column::{range_histogram, Column, FilterEditorKind, CHOICE_LIMIT},
    engine::{CancelToken, ViewEngine},
    file, format,
    numeric::Numeric,
    predicate::{Predicate, CHUNK},
    scope::{for_rows, RowScope, Selection},
    value_index::{count_values, ValueIndex, INDEX_LIMIT},
    ColumnCapabilities, ColumnInfo, ColumnSummary, DataKind, DistinctResult, Error, Export,
    ExportOptions, FileInfo, FileType, Filter, Histogram, OpenOptions, Result, RowBlock, ViewInfo,
    ViewState,
};

/// One open file. Safe to share between threads.
pub struct Document {
    info: FileInfo,
    source_path: PathBuf,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    n_rows: usize,
    indexes: Vec<OnceLock<Indexed>>,
    filter_editors: Vec<OnceLock<FilterEditorKind>>,
    views: ViewEngine,
}

/// A column's value index, built on first use.
enum Indexed {
    Built(Arc<ValueIndex>),
    /// More than [`INDEX_LIMIT`] distinct values: counted on demand instead.
    TooMany,
}

impl Document {
    /// Open a supported file using its format's default options.
    /// CSV/TSV and XLSX use the first record as headers; XLSX opens its first sheet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, &OpenOptions::default())
    }

    /// Open a file with an explicit format, header policy or XLSX sheet name.
    pub fn open_with_options(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        let source_path = path.canonicalize()?;
        let file_size = std::fs::metadata(path)?.len();
        let (format, loaded) = file::read(path, options)?;
        let batch = loaded.batch;
        let schema = batch.schema();

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
            file_size,
            format,
            metadata: loaded.metadata,
        };

        let columns = batch.columns().to_vec();
        let indexes = (0..columns.len()).map(|_| OnceLock::new()).collect();

        let filter_editors = (0..columns.len()).map(|_| OnceLock::new()).collect();

        Ok(Self {
            info,
            source_path,
            schema,
            columns,
            n_rows,
            indexes,
            filter_editors,
            views: ViewEngine::new(n_rows),
        })
    }

    pub fn info(&self) -> &FileInfo {
        &self.info
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Parse text once at the input boundary, retaining the column's full type.
    pub fn parse_bound(&self, column: u32, text: &str) -> Result<crate::Bound> {
        crate::Bound::parse(Column::new(self.column(column)?).logical_data_type(), text)
    }

    pub fn make_bound_i64(&self, column: u32, value: i64) -> Result<crate::Bound> {
        crate::Bound::from_i64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    pub fn make_bound_u64(&self, column: u32, value: u64) -> Result<crate::Bound> {
        crate::Bound::from_u64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    pub fn make_bound_f64(&self, column: u32, value: f64) -> Result<crate::Bound> {
        crate::Bound::from_f64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    /// Decimal coefficient is transported as text to accommodate 256-bit values.
    pub fn make_bound_decimal(
        &self,
        column: u32,
        coefficient: &str,
        scale: i8,
    ) -> Result<crate::Bound> {
        let coefficient = coefficient
            .parse::<arrow::datatypes::i256>()
            .map_err(|_| Error::InvalidArgument("invalid decimal coefficient".into()))?;
        crate::Bound::from_decimal(
            Column::new(self.column(column)?).logical_data_type(),
            coefficient,
            scale,
        )
    }

    /// Describe the filter editor using whole-column cardinality. Facet counts
    /// respect other columns' filters and search without changing the editor.
    pub fn filter_spec(&self, column: u32, state: Option<&ViewState>) -> Result<crate::FilterSpec> {
        use crate::{FilterEditor, FilterSpec};
        let array = self.column(column)?;
        let typed = Column::new(array);
        let cell = &self.filter_editors[column as usize];
        let editor_kind = match cell.get() {
            Some(kind) => *kind,
            None => {
                let supported =
                    arrow::row::RowConverter::supports_fields(&[arrow::row::SortField::new(
                        array.data_type().clone(),
                    )]);
                let low_cardinality = if !supported {
                    false
                } else {
                    match self.indexes[column as usize].get() {
                        Some(Indexed::Built(index)) => index.len() <= CHOICE_LIMIT,
                        Some(Indexed::TooMany) => false,
                        None => crate::value_index::has_at_most(array, CHOICE_LIMIT)?,
                    }
                };
                *cell.get_or_init(|| typed.filter_editor(low_cardinality))
            }
        };
        let editor = match editor_kind {
            FilterEditorKind::Choices => {
                let rows = self.rows_in(RowScope::facet(state, column))?;
                FilterEditor::Choices {
                    values: self.keyed_index(column)?.counts_all(&rows),
                }
            }
            FilterEditorKind::Range => {
                let rows = self.rows_in(RowScope::facet(state, column))?;
                FilterEditor::Range {
                    kind: DataKind::of(typed.logical_data_type()),
                    distribution: range_histogram::build(array, &rows, 60)?,
                }
            }
            FilterEditorKind::Contains => FilterEditor::Contains,
            FilterEditorKind::NullOnly => FilterEditor::NullOnly,
        };
        Ok(FilterSpec {
            nullable: self.info.columns[column as usize].nullable,
            editor,
        })
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
        format: FileType,
        options: ExportOptions,
    ) -> Result<Export> {
        format.require_enabled()?;
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
        if format == FileType::Xlsx && !self.nested_export_columns(columns)?.is_empty() {
            return Err(Error::Unsupported("XLSX export does not support nested columns; use Parquet or CSV/TSV with JSON conversion".into()));
        }
        if matches!(format, FileType::Csv | FileType::Tsv) && !options.convert_nested_to_json {
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

        let arrays: Vec<&ArrayRef> = columns.iter().map(|&c| &self.columns[c as usize]).collect();
        let formatters = arrays
            .iter()
            .map(|a| Column::new(a).formatter())
            .collect::<Result<Vec<_>>>()?;

        let mut cells = Vec::with_capacity(source_rows.len() * columns.len());
        for &r in source_rows {
            for (a, f) in arrays.iter().zip(&formatters) {
                cells.push(format::cell(a.as_ref(), f, r as usize));
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
                let block = take(column.as_ref(), &indices, None)?;
                if let Some(predicate) = Column::new(&block).prepare_search(text)? {
                    predicate.evaluate(0..block.len(), &mut hits, &token)?;
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
    ///
    /// A column with at most 100,000 distinct values is indexed: the result
    /// is the first `limit` values in value order, and their keys work in
    /// [`Filter::InValues`]. A column with more is counted without an index:
    /// the result is the `limit` most common values (still in value order),
    /// `keyed` is false and the keys only number them.
    pub fn distinct_values(
        &self,
        column: u32,
        filters: Option<&ViewState>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let index = self.index(column)?;
        let rows = self.rows_in(RowScope::facet(filters, column))?;
        let limit = limit as usize;
        Ok(match index {
            Some(idx) => {
                let present = idx.counts(&rows);
                DistinctResult {
                    total: present.len() as u64,
                    truncated: present.len() > limit,
                    values: present.into_iter().take(limit).collect(),
                    keyed: true,
                }
            }
            None => {
                let counted = count_values(self.column(column)?, &rows, limit)?;
                DistinctResult {
                    total: counted.distinct,
                    truncated: counted.distinct > limit as u64,
                    values: counted.top,
                    keyed: false,
                }
            }
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
        for_rows!(&rows, |it| numeric.histogram(it, bins))
    }

    /// Distribution over the displayed rows, including this column's own
    /// filters and the text search. The axis still spans the source column.
    pub fn view_histogram(&self, version: u64, column: u32, bins: u32) -> Result<Histogram> {
        let numeric = self.numeric(column)?;
        let view = self.views.snapshot(version)?;
        let rows = self.rows_in(RowScope::Rows(view.rows.clone()))?;
        for_rows!(&rows, |it| numeric.histogram(it, bins))
    }

    /// Display text for a raw value of a date, time, timestamp or duration
    /// column (its integer physical value, as used by histograms), formatted
    /// exactly like the table's cells. Also works for
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

    /// Operations supported by this column's logical type.
    pub fn column_capabilities(&self, column: u32) -> Result<ColumnCapabilities> {
        Ok(Column::new(self.column(column)?).capabilities())
    }

    /// Validate and prepare an explicit filter without scanning data. Value-list
    /// filters may build the column's lazy index; other filters only parse scalars.
    pub fn validate_filter(&self, filter: &Filter) -> Result<()> {
        self.prepare_filter(filter).map(|_| ())
    }

    /// Keys for the given displayed labels (`None` = null). Unknown labels
    /// are ignored. Used by the CLI and MCP, which filter by value text.
    pub fn keys_for_labels(&self, column: u32, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let idx = self.keyed_index(column)?;
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
        let array = self.column(column)?;
        let rows = self.rows_in(RowScope::Rows(self.views.current().rows.clone()))?;
        let mut s = ColumnSummary {
            rows: rows.len() as u64,
            null_count: 0,
            nan_count: 0,
            distinct_count: 0,
            min: None,
            max: None,
            min_label: None,
            max_label: None,
            mean: None,
            std_dev: None,
            top_values: Vec::new(),
        };
        let f = format::formatter(array.as_ref())?;
        let label = |row: Option<usize>| row.and_then(|r| format::cell(array.as_ref(), &f, r));
        if let Some(numeric) = Numeric::new(array) {
            let (lo, hi) = for_rows!(&rows, |it| numeric.summarize(it, &mut s));
            (s.min_label, s.max_label) = (label(lo), label(hi));
            return Ok(s);
        }

        let top = top as usize;
        s.null_count = match &rows {
            Selection::All(_) => array.null_count() as u64,
            Selection::Rows(_) => rows.iter().filter(|&r| array.is_null(r)).count() as u64,
        };
        let mut values = match self.index(column)? {
            Some(idx) => {
                let values = idx.counts(&rows);
                s.distinct_count = values.len() as u64;
                // Value order: the first and last non-null are the extremes.
                let mut present = values.iter().filter_map(|v| v.label.as_ref());
                s.min_label = present.next().cloned();
                s.max_label = present.next_back().cloned();
                values
            }
            None => {
                // One spare in case null is among the most common.
                let counted = count_values(array, &rows, top + 1)?;
                s.distinct_count = counted.distinct;
                let (lo, hi) = extremes(array, &rows)?;
                (s.min_label, s.max_label) = (label(lo), label(hi));
                counted.top
            }
        };
        if s.null_count > 0 {
            s.distinct_count -= 1;
        }
        values.retain(|v| v.label.is_some());
        // Stable: ties stay in value order.
        values.sort_by_key(|v| std::cmp::Reverse(v.count));
        values.truncate(top);
        s.top_values = values;
        Ok(s)
    }

    // ---- internals ----

    fn column(&self, c: u32) -> Result<&ArrayRef> {
        self.columns
            .get(c as usize)
            .ok_or_else(|| Error::InvalidArgument(format!("no column {c}")))
    }

    /// The column's value index, or `None` if it has too many values.
    fn index(&self, c: u32) -> Result<Option<Arc<ValueIndex>>> {
        let array = self.column(c)?;
        let cell = &self.indexes[c as usize];
        let found = match cell.get() {
            Some(found) => found,
            None => {
                let built = match ValueIndex::build(array, INDEX_LIMIT)? {
                    Some(idx) => Indexed::Built(Arc::new(idx)),
                    None => Indexed::TooMany,
                };
                cell.get_or_init(|| built)
            }
        };
        Ok(match found {
            Indexed::Built(idx) => Some(idx.clone()),
            Indexed::TooMany => None,
        })
    }

    /// The column's value index, for filtering by key.
    fn keyed_index(&self, c: u32) -> Result<Arc<ValueIndex>> {
        self.index(c)?.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "column {} has more than {INDEX_LIMIT} distinct values; filter it with contains or a range",
                self.info.columns[c as usize].name
            ))
        })
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
            // A view keeps each row at most once, so a full-length view is
            // every row, and counting in file order is faster.
            RowScope::Rows(rows) if rows.len() == self.n_rows => Selection::All(self.n_rows),
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

    /// Prepare column predicates once, then combine AND/OR in bounded batches.
    /// Masks are reused across columns and batches, not retained per column.
    fn select(
        &self,
        filters: &[Filter],
        search: Option<&str>,
        skip: Option<u32>,
        token: &CancelToken,
    ) -> Result<Vec<u32>> {
        token.check()?;
        let filters = filters
            .iter()
            .filter(|f| Some(f.column()) != skip)
            .map(|f| self.prepare_filter(f))
            .collect::<Result<Vec<_>>>()?;
        let search = search.filter(|s| !s.trim().is_empty());
        let search_tests = match search {
            Some(text) => self
                .columns
                .iter()
                .map(|a| Column::new(a).prepare_search(text))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        if filters.is_empty() && search.is_none() {
            let rows = (0..self.n_rows as u32).collect();
            token.check()?;
            return Ok(rows);
        }
        let batch_len = self.n_rows.min(CHUNK);
        let mut selected = vec![true; batch_len];
        let mut scratch = vec![false; batch_len];
        let mut searched = vec![false; if search.is_some() { batch_len } else { 0 }];
        let mut rows = Vec::new();
        for start in (0..self.n_rows).step_by(CHUNK) {
            token.check()?;
            let end = (start + CHUNK).min(self.n_rows);
            let range = start..end;
            let selected = &mut selected[..range.len()];
            let scratch = &mut scratch[..range.len()];
            selected.fill(true);
            for test in &filters {
                test.intersect(range.clone(), selected, scratch, token)?;
                if !selected.contains(&true) {
                    break;
                }
            }
            if !selected.contains(&true) {
                continue;
            }
            if search.is_some() {
                let searched = &mut searched[..range.len()];
                searched.fill(false);
                for test in &search_tests {
                    test.evaluate(range.clone(), scratch, token)?;
                    for (hit, other) in searched.iter_mut().zip(scratch.iter()) {
                        *hit |= *other;
                    }
                }
                for (hit, other) in selected.iter_mut().zip(searched) {
                    *hit &= *other;
                }
            }
            rows.extend(
                range
                    .zip(selected)
                    .filter_map(|(r, hit)| (*hit).then_some(r as u32)),
            );
        }
        token.check()?;
        Ok(rows)
    }

    fn prepare_filter(&self, filter: &Filter) -> Result<Predicate<'_>> {
        let column = Column::new(self.column(filter.column())?);
        match filter {
            Filter::InValues { keys, .. } => {
                Ok(column.prepare_values(self.keyed_index(filter.column())?, keys))
            }
            _ => column.prepare_filter(filter),
        }
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

/// Rows holding the smallest and largest non-null values among `rows`, in
/// the column's own order. `None` for types Arrow cannot compare. (Numeric
/// columns get theirs from the summary, which also leaves out NaN.)
fn extremes(array: &ArrayRef, rows: &Selection) -> Result<(Option<usize>, Option<usize>)> {
    let Ok(cmp) = make_comparator(array.as_ref(), array.as_ref(), SortOptions::default()) else {
        return Ok((None, None));
    };
    let (mut lo, mut hi) = (None::<usize>, None::<usize>);
    for r in rows.iter().filter(|&r| array.is_valid(r)) {
        if lo.is_none_or(|l| cmp(r, l).is_lt()) {
            lo = Some(r);
        }
        if hi.is_none_or(|h| cmp(r, h).is_gt()) {
            hi = Some(r);
        }
    }
    Ok((lo, hi))
}
