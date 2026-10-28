//! UniFFI bindings of gewu-core for the macOS app.
//!
//! All methods are synchronous. The app calls the slow ones (`open`,
//! `apply_view`, `distinct_values`, `column_stats`) from a background task;
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

#[derive(uniffi::Record)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
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

#[derive(uniffi::Record)]
pub struct ColumnStats {
    pub null_count: u64,
    pub distinct_count: u64,
    pub min: Option<String>,
    pub max: Option<String>,
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
                    nullable: c.nullable,
                })
                .collect(),
            row_groups: i.row_groups,
            file_size: i.file_size,
            compression: i.compression.clone(),
        }
    }

    pub fn current_view(&self) -> ViewInfo {
        self.inner.current_view().into()
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
        let h = self.inner.histogram(column, state.as_ref(), bins)?;
        Ok(Histogram {
            lo: h.lo,
            hi: h.hi,
            integer: h.integer,
            counts: h.counts,
            null_count: h.null_count,
            nan_count: h.nan_count,
            total: h.total,
        })
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

    pub fn column_stats(&self, column: u32) -> Result<ColumnStats, GewuError> {
        let s = self.inner.column_stats(column)?;
        Ok(ColumnStats {
            null_count: s.null_count,
            distinct_count: s.distinct_count,
            min: s.min,
            max: s.max,
        })
    }
}
