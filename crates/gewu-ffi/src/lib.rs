//! UniFFI bindings of gewu-core for the macOS app.
//!
//! All methods are synchronous. The app calls the slow ones (`open`,
//! `apply_view`, `distinct_values`, `column_summary`) from a background task;
//! a newer `apply_view` cancels an older one inside the core, so the app does
//! not need to manage cancellation itself. `rows` is fast and is called from
//! the main thread.

use std::sync::Arc;

use gewu_core as core;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum GewuError {
    #[error("{message}")]
    Io { message: String },
    #[error("{message}")]
    Unsupported { message: String },
    #[error("{message}")]
    InvalidArgument { message: String },
    #[error("cancelled")]
    Cancelled,
    #[error("stale view version {requested}, current is {current}")]
    StaleVersion { requested: u64, current: u64 },
    #[error("{message}")]
    Internal { message: String },
}

impl From<core::Error> for GewuError {
    fn from(e: core::Error) -> Self {
        match e {
            core::Error::Io(e) => GewuError::Io {
                message: e.to_string(),
            },
            core::Error::Unsupported(message) => GewuError::Unsupported { message },
            core::Error::InvalidArgument(message) => GewuError::InvalidArgument { message },
            core::Error::Cancelled => GewuError::Cancelled,
            core::Error::StaleVersion { requested, current } => {
                GewuError::StaleVersion { requested, current }
            }
            other => GewuError::Internal {
                message: other.to_string(),
            },
        }
    }
}

/// How gewu treats a column's values; mirrors `gewu_core::DataKind`.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    Text,
    Integer,
    Float,
    Decimal,
    Boolean,
    Date,
    Time,
    Timestamp,
    Duration,
    Interval,
    Binary,
    List,
    Other,
}

impl From<core::DataKind> for DataKind {
    fn from(k: core::DataKind) -> Self {
        use core::DataKind as K;
        match k {
            K::Text => Self::Text,
            K::Integer => Self::Integer,
            K::Float => Self::Float,
            K::Decimal => Self::Decimal,
            K::Boolean => Self::Boolean,
            K::Date => Self::Date,
            K::Time => Self::Time,
            K::Timestamp => Self::Timestamp,
            K::Duration => Self::Duration,
            K::Interval => Self::Interval,
            K::Binary => Self::Binary,
            K::List => Self::List,
            K::Other => Self::Other,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub kind: DataKind,
    /// Range filter and histogram apply (numbers, dates, times).
    pub ordered: bool,
    pub nullable: bool,
}

#[derive(uniffi::Record)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub row_groups: u32,
    pub file_size: u64,
    pub compression: Vec<String>,
    pub created_by: Option<String>,
    /// Footer key-value metadata (Arrow's own schema left out).
    pub metadata: Vec<KeyValue>,
}

#[derive(uniffi::Record)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

/// One column over the current view's rows, for the inspector.
#[derive(uniffi::Record)]
pub struct ColumnSummary {
    pub rows: u64,
    pub null_count: u64,
    pub nan_count: u64,
    pub distinct_count: u64,
    /// Numbers, dates and times (dates and times as raw integer values).
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Smallest and largest values of any comparable column, as displayed.
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    /// Other columns: most common values first.
    pub top_values: Vec<DistinctValue>,
}

#[derive(uniffi::Record)]
pub struct SortKey {
    pub column: u32,
    pub descending: bool,
}

/// One end of a range filter. Integers stay exact.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum Bound {
    Int { value: i64 },
    UInt { value: u64 },
    Float { value: f64 },
}

#[derive(uniffi::Enum)]
pub enum Filter {
    InValues {
        column: u32,
        keys: Vec<u64>,
    },
    IsNull {
        column: u32,
        negate: bool,
    },
    Contains {
        column: u32,
        text: String,
        case_sensitive: bool,
    },
    /// Inclusive; a `nil` bound is open. Numeric, date and time columns.
    Range {
        column: u32,
        min: Option<Bound>,
        max: Option<Bound>,
    },
}

#[derive(uniffi::Record)]
pub struct ViewState {
    pub sort: Vec<SortKey>,
    pub filters: Vec<Filter>,
    pub search: Option<String>,
}

#[derive(uniffi::Record)]
pub struct ViewInfo {
    pub version: u64,
    pub row_count: u64,
}

#[derive(uniffi::Record)]
pub struct RowBlock {
    pub version: u64,
    pub start: u64,
    pub n_rows: u32,
    pub n_cols: u32,
    /// Row-major, `n_rows * n_cols` cells; `nil` is null.
    pub cells: Vec<Option<String>>,
    pub source_rows: Vec<u64>,
    /// With a search active, row-major flags for cells that match it.
    pub matched: Vec<bool>,
}

#[derive(uniffi::Record)]
pub struct DistinctValue {
    pub key: u64,
    pub label: Option<String>,
    pub count: u64,
}

#[derive(uniffi::Record)]
pub struct DistinctResult {
    pub values: Vec<DistinctValue>,
    pub total: u64,
    pub truncated: bool,
    /// False when the column has too many values to index: the keys then
    /// cannot be used in an `in_values` filter.
    pub keyed: bool,
}

/// Distribution of a numeric column. Bins split `[lo, hi)` evenly.
#[derive(uniffi::Record)]
pub struct Histogram {
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub integer: bool,
    pub counts: Vec<u64>,
    pub null_count: u64,
    pub nan_count: u64,
    pub total: u64,
}

impl From<ViewState> for core::ViewState {
    fn from(s: ViewState) -> Self {
        core::ViewState {
            sort: s
                .sort
                .into_iter()
                .map(|k| core::SortKey {
                    column: k.column,
                    descending: k.descending,
                })
                .collect(),
            filters: s
                .filters
                .into_iter()
                .map(|f| match f {
                    Filter::InValues { column, keys } => core::Filter::InValues { column, keys },
                    Filter::IsNull { column, negate } => core::Filter::IsNull { column, negate },
                    Filter::Contains {
                        column,
                        text,
                        case_sensitive,
                    } => core::Filter::Contains {
                        column,
                        text,
                        case_sensitive,
                    },
                    Filter::Range { column, min, max } => core::Filter::Range {
                        column,
                        min: min.map(Into::into),
                        max: max.map(Into::into),
                    },
                })
                .collect(),
            search: s.search,
        }
    }
}

impl From<Bound> for core::Bound {
    fn from(b: Bound) -> Self {
        match b {
            Bound::Int { value } => core::Bound::Int(value),
            Bound::UInt { value } => core::Bound::UInt(value),
            Bound::Float { value } => core::Bound::Float(value),
        }
    }
}

impl From<core::ViewInfo> for ViewInfo {
    fn from(v: core::ViewInfo) -> Self {
        ViewInfo {
            version: v.version,
            row_count: v.row_count,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum ExportFormat {
    Csv,
    Tsv,
    Parquet,
}

impl From<ExportFormat> for core::ExportFormat {
    fn from(value: ExportFormat) -> Self {
        match value {
            ExportFormat::Csv => Self::Csv,
            ExportFormat::Tsv => Self::Tsv,
            ExportFormat::Parquet => Self::Parquet,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ExportOptions {
    pub convert_nested_to_json: bool,
}

#[derive(uniffi::Record)]
pub struct ExportProgress {
    pub rows_written: u64,
    pub total_rows: u64,
}

/// An immutable view snapshot with independent cancellation. Single-use.
#[derive(uniffi::Object)]
pub struct GewuExport {
    inner: core::Export,
}

#[uniffi::export]
impl GewuExport {
    /// Write the captured result. Slow: call off the main thread.
    pub fn write(&self, path: String) -> Result<(), GewuError> {
        Ok(self.inner.write(path)?)
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn progress(&self) -> ExportProgress {
        let progress = self.inner.progress();
        ExportProgress {
            rows_written: progress.rows_written,
            total_rows: progress.total_rows,
        }
    }
}

impl From<core::Histogram> for Histogram {
    fn from(h: core::Histogram) -> Self {
        Self {
            lo: h.lo,
            hi: h.hi,
            integer: h.integer,
            counts: h.counts,
            null_count: h.null_count,
            nan_count: h.nan_count,
            total: h.total,
        }
    }
}

/// One open file.
#[derive(uniffi::Object)]
pub struct GewuDocument {
    inner: core::Document,
}

#[uniffi::export]
impl GewuDocument {
    /// Open and decode a Parquet file. Slow: call off the main thread.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: core::Document::open(path)?,
        }))
    }

    pub fn info(&self) -> FileInfo {
        let i = self.inner.info();
        FileInfo {
            path: i.path.clone(),
            row_count: i.row_count,
            columns: i
                .columns
                .iter()
                .map(|c| ColumnInfo {
                    name: c.name.clone(),
                    type_name: c.type_name.clone(),
                    kind: c.kind.into(),
                    ordered: c.kind.is_ordered(),
                    nullable: c.nullable,
                })
                .collect(),
            row_groups: i.row_groups,
            file_size: i.file_size,
            compression: i.compression.clone(),
            created_by: i.created_by.clone(),
            metadata: i
                .metadata
                .iter()
                .map(|(key, value)| KeyValue {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect(),
        }
    }

    pub fn current_view(&self) -> ViewInfo {
        self.inner.current_view().into()
    }

    /// Capture the current result and displayed column order without copying
    /// cells. Later queries do not change this export's contents.
    pub fn prepare_export(
        &self,
        version: u64,
        columns: Vec<u32>,
        format: ExportFormat,
        options: ExportOptions,
    ) -> Result<Arc<GewuExport>, GewuError> {
        Ok(Arc::new(GewuExport {
            inner: self.inner.prepare_export(
                version,
                &columns,
                format.into(),
                core::ExportOptions {
                    convert_nested_to_json: options.convert_nested_to_json,
                },
            )?,
        }))
    }

    pub fn nested_export_columns(&self, columns: Vec<u32>) -> Result<Vec<String>, GewuError> {
        Ok(self.inner.nested_export_columns(&columns)?)
    }

    /// Sort and filter. Slow: call off the main thread. A newer call makes
    /// this one return `Cancelled`.
    pub fn apply_view(&self, state: ViewState) -> Result<ViewInfo, GewuError> {
        Ok(self.inner.apply_view(&state.into())?.into())
    }

    pub fn cancel_pending(&self) {
        self.inner.cancel_pending();
    }

    /// Formatted cells for display rows `start..start+count`. Fast.
    pub fn rows(
        &self,
        version: u64,
        start: u64,
        count: u32,
        columns: Vec<u32>,
    ) -> Result<RowBlock, GewuError> {
        let b = self.inner.rows(version, start, count, &columns)?;
        Ok(RowBlock {
            version: b.version,
            start: b.start,
            n_rows: b.n_rows,
            n_cols: b.n_cols,
            cells: b.cells,
            source_rows: b.source_rows,
            matched: b.matched,
        })
    }

    /// Distinct values with counts, for the checkbox filter. Pass the current
    /// view state to count only rows that pass filters on other columns.
    pub fn distinct_values(
        &self,
        column: u32,
        state: Option<ViewState>,
        limit: u32,
    ) -> Result<DistinctResult, GewuError> {
        let state: Option<core::ViewState> = state.map(Into::into);
        let d = self.inner.distinct_values(column, state.as_ref(), limit)?;
        Ok(DistinctResult {
            values: d
                .values
                .into_iter()
                .map(|v| DistinctValue {
                    key: v.key,
                    label: v.label,
                    count: v.count,
                })
                .collect(),
            total: d.total,
            truncated: d.truncated,
            keyed: d.keyed,
        })
    }

    /// Distribution for the range filter. Pass the current view state to
    /// count only rows that pass filters on other columns. Slow on big files:
    /// call off the main thread.
    pub fn histogram(
        &self,
        column: u32,
        state: Option<ViewState>,
        bins: u32,
    ) -> Result<Histogram, GewuError> {
        let state: Option<core::ViewState> = state.map(Into::into);
        Ok(self.inner.histogram(column, state.as_ref(), bins)?.into())
    }

    /// Distribution over the current displayed rows, with all filters applied.
    /// Call off the main thread.
    pub fn view_histogram(
        &self,
        version: u64,
        column: u32,
        bins: u32,
    ) -> Result<Histogram, GewuError> {
        Ok(self.inner.view_histogram(version, column, bins)?.into())
    }

    /// Display text for a raw value of a date, time or timestamp column
    /// (as in histogram axes and range bounds), formatted like its cells.
    pub fn format_value(&self, column: u32, value: i64) -> Result<String, GewuError> {
        Ok(self.inner.format_value(column, value)?)
    }

    /// Read user-typed text (a number or a date/time) as a range bound.
    pub fn parse_value(&self, column: u32, text: String) -> Result<Bound, GewuError> {
        Ok(match self.inner.parse_value(column, &text)? {
            core::Bound::Int(value) => Bound::Int { value },
            core::Bound::UInt(value) => Bound::UInt { value },
            core::Bound::Float(value) => Bound::Float { value },
        })
    }

    /// Summary of a column over the current view (filters and search apply).
    /// Call off the main thread.
    pub fn column_summary(&self, column: u32, top: u32) -> Result<ColumnSummary, GewuError> {
        let s = self.inner.column_summary(column, top)?;
        Ok(ColumnSummary {
            rows: s.rows,
            null_count: s.null_count,
            nan_count: s.nan_count,
            distinct_count: s.distinct_count,
            min: s.min,
            max: s.max,
            min_label: s.min_label,
            max_label: s.max_label,
            mean: s.mean,
            std_dev: s.std_dev,
            top_values: s
                .top_values
                .into_iter()
                .map(|v| DistinctValue {
                    key: v.key,
                    label: v.label,
                    count: v.count,
                })
                .collect(),
        })
    }
}
