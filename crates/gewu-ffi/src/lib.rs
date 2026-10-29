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
pub struct ColumnCapabilities {
    pub contains: bool,
    pub range: bool,
    pub equals: bool,
    pub histogram: bool,
}

impl From<core::ColumnCapabilities> for ColumnCapabilities {
    fn from(capabilities: core::ColumnCapabilities) -> Self {
        Self {
            contains: capabilities.contains,
            range: capabilities.range,
            equals: capabilities.equals,
            histogram: capabilities.histogram,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub kind: DataKind,
    pub capabilities: ColumnCapabilities,
    pub nullable: bool,
}

#[derive(uniffi::Record)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub format: String,
    pub row_groups: Option<u32>,
    pub file_size: u64,
    pub compression: Vec<String>,
    pub created_by: Option<String>,
    /// Footer key-value metadata (Arrow's own schema left out).
    pub metadata: Vec<KeyValue>,
    pub sheet: Option<String>,
    pub sheets: Vec<String>,
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
    /// Exact typed equality; parsing is owned by the column's type.
    Equals {
        column: u32,
        value: String,
    },
    /// Inclusive typed bounds. A missing endpoint is open.
    Range {
        column: u32,
        min: Option<Arc<Bound>>,
        max: Option<Arc<Bound>>,
    },
}

/// An immutable, type-checked range endpoint owned by the core.
#[derive(uniffi::Object)]
pub struct Bound {
    inner: core::Bound,
}

impl From<core::Bound> for Bound {
    fn from(inner: core::Bound) -> Self {
        Self { inner }
    }
}

#[uniffi::export]
impl Bound {
    pub fn display(&self) -> String {
        self.inner.display().to_owned()
    }

    /// Approximate coordinate for a chart; never use it to rebuild a filter.
    pub fn axis_value(&self) -> f64 {
        self.inner.axis_value()
    }

    /// Compare typed endpoints exactly, independently of their display text.
    pub fn same_value(&self, other: Arc<Bound>) -> bool {
        self.inner.same_value(&other.inner)
    }
}

#[derive(uniffi::Record)]
pub struct RangeBounds {
    pub min: Option<Arc<Bound>>,
    pub max: Option<Arc<Bound>>,
}

#[derive(uniffi::Record)]
pub struct RangeHistogram {
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
    pub min_label: String,
    pub max_label: String,
}

#[derive(uniffi::Enum)]
pub enum FilterEditor {
    Range {
        kind: DataKind,
        distribution: Option<RangeHistogram>,
    },
    Choices {
        values: Vec<DistinctValue>,
    },
    Contains,
    NullOnly,
}

#[derive(uniffi::Record)]
pub struct FilterSpec {
    pub nullable: bool,
    pub editor: FilterEditor,
}

impl From<core::FilterSpec> for FilterSpec {
    fn from(spec: core::FilterSpec) -> Self {
        let editor = match spec.editor {
            core::FilterEditor::Range { kind, distribution } => FilterEditor::Range {
                kind: kind.into(),
                distribution: distribution.map(|distribution| RangeHistogram {
                    histogram: distribution.histogram.into(),
                    min_label: distribution.min_label,
                    max_label: distribution.max_label,
                    ranges: distribution
                        .ranges
                        .into_iter()
                        .map(|bounds| RangeBounds {
                            min: bounds.min.map(|bound| Arc::new(bound.into())),
                            max: bounds.max.map(|bound| Arc::new(bound.into())),
                        })
                        .collect(),
                }),
            },
            core::FilterEditor::Choices { values } => FilterEditor::Choices {
                values: values
                    .into_iter()
                    .map(|value| DistinctValue {
                        key: value.key,
                        label: value.label,
                        count: value.count,
                    })
                    .collect(),
            },
            core::FilterEditor::Contains => FilterEditor::Contains,
            core::FilterEditor::NullOnly => FilterEditor::NullOnly,
        };
        Self {
            nullable: spec.nullable,
            editor,
        }
    }
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
            filters: s.filters.into_iter().map(Into::into).collect(),
            search: s.search,
        }
    }
}

impl From<Filter> for core::Filter {
    fn from(filter: Filter) -> Self {
        match filter {
            Filter::InValues { column, keys } => Self::InValues { column, keys },
            Filter::IsNull { column, negate } => Self::IsNull { column, negate },
            Filter::Contains {
                column,
                text,
                case_sensitive,
            } => Self::Contains {
                column,
                text,
                case_sensitive,
            },
            Filter::Equals { column, value } => Self::Equals { column, value },
            Filter::Range { column, min, max } => Self::Range {
                column,
                min: min.map(|bound| bound.inner.clone()),
                max: max.map(|bound| bound.inner.clone()),
            },
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
pub enum FileType {
    Csv,
    Tsv,
    Parquet,
    Xlsx,
}

impl From<FileType> for core::FileType {
    fn from(value: FileType) -> Self {
        match value {
            FileType::Csv => Self::Csv,
            FileType::Tsv => Self::Tsv,
            FileType::Parquet => Self::Parquet,
            FileType::Xlsx => Self::Xlsx,
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
    /// Open a supported table file; XLSX defaults to its first worksheet.
    /// Slow: call off the main thread.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, GewuError> {
        Ok(Arc::new(Self {
            inner: core::Document::open(path)?,
        }))
    }

    pub fn info(&self) -> FileInfo {
        let i = self.inner.info();
        let mut info = FileInfo {
            path: i.path.clone(),
            row_count: i.row_count,
            columns: i
                .columns
                .iter()
                .enumerate()
                .map(|(column, c)| {
                    let capabilities = self
                        .inner
                        .column_capabilities(column as u32)
                        .expect("column came from this document's schema");
                    ColumnInfo {
                        name: c.name.clone(),
                        type_name: c.type_name.clone(),
                        kind: c.kind.into(),
                        capabilities: capabilities.into(),
                        nullable: c.nullable,
                    }
                })
                .collect(),
            format: i.format.to_string(),
            row_groups: None,
            file_size: i.file_size,
            compression: Vec::new(),
            created_by: None,
            metadata: Vec::new(),
            sheet: None,
            sheets: Vec::new(),
        };
        match &i.metadata {
            core::FileMetadata::Parquet {
                row_groups,
                compression,
                created_by,
                key_values,
            } => {
                info.row_groups = Some(*row_groups);
                info.compression = compression.clone();
                info.created_by = created_by.clone();
                info.metadata = key_values
                    .iter()
                    .map(|(key, value)| KeyValue {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect();
            }
            core::FileMetadata::Delimited => {}
            core::FileMetadata::Xlsx { sheet, sheets } => {
                info.sheet = Some(sheet.clone());
                info.sheets = sheets.clone();
            }
        }
        info
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
        format: FileType,
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

    /// The core selects the editor and supplies its values or typed histogram bins.
    /// Slow: call off the main thread.
    pub fn filter_spec(
        &self,
        column: u32,
        state: Option<ViewState>,
    ) -> Result<FilterSpec, GewuError> {
        let state = state.map(Into::into);
        Ok(self.inner.filter_spec(column, state.as_ref())?.into())
    }

    pub fn parse_bound(&self, column: u32, text: String) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.parse_bound(column, &text)?.into()))
    }

    pub fn make_bound_int(&self, column: u32, value: i64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_i64(column, value)?.into()))
    }

    pub fn make_bound_uint(&self, column: u32, value: u64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_u64(column, value)?.into()))
    }

    pub fn make_bound_float(&self, column: u32, value: f64) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(self.inner.make_bound_f64(column, value)?.into()))
    }

    /// Decimal coefficient transported as an integer string to preserve all digits.
    pub fn make_bound_decimal(
        &self,
        column: u32,
        coefficient: String,
        scale: i8,
    ) -> Result<Arc<Bound>, GewuError> {
        Ok(Arc::new(
            self.inner
                .make_bound_decimal(column, &coefficient, scale)?
                .into(),
        ))
    }

    /// Validate a filter without changing the current view.
    pub fn validate_filter(&self, filter: Filter) -> Result<(), GewuError> {
        Ok(self.inner.validate_filter(&filter.into())?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_filters_validate_and_execute_across_ffi() {
        let source =
            std::env::temp_dir().join(format!("gewu-ffi-typed-{}.csv", std::process::id()));
        std::fs::write(&source, "id\n9007199254740992\n9007199254740993\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        assert!(doc.info().columns[0].capabilities.histogram);
        assert!(!doc.info().columns[0].capabilities.contains);
        // Chart coordinates may round a large ID; typed filters below must not.
        let large = doc.parse_bound(0, "9007199254740993".into()).unwrap();
        assert_eq!(large.display(), "9007199254740993");
        assert_eq!(large.axis_value(), 9_007_199_254_740_992.0);
        assert!(large.same_value(doc.make_bound_int(0, 9_007_199_254_740_993).unwrap()));
        assert!(!large.same_value(doc.make_bound_int(0, 9_007_199_254_740_992).unwrap()));
        assert!(matches!(
            doc.parse_bound(0, "NaN".into()),
            Err(GewuError::InvalidArgument { .. })
        ));
        let before = doc.current_view();
        assert!(matches!(
            doc.parse_bound(0, "not a number".into()),
            Err(GewuError::InvalidArgument { .. })
        ));
        assert!(matches!(
            doc.filter_spec(0, None).unwrap().editor,
            FilterEditor::Choices { .. }
        ));
        assert_eq!(doc.current_view().version, before.version);

        for filter in [
            Filter::Equals {
                column: 0,
                value: "9007199254740993".into(),
            },
            Filter::Range {
                column: 0,
                min: Some(large),
                max: None,
            },
            Filter::Range {
                column: 0,
                min: Some(doc.make_bound_int(0, 9_007_199_254_740_993).unwrap()),
                max: None,
            },
        ] {
            let view = doc
                .apply_view(ViewState {
                    sort: Vec::new(),
                    filters: vec![filter],
                    search: None,
                })
                .unwrap();
            let rows = doc.rows(view.version, 0, 10, vec![0]).unwrap();
            assert_eq!(rows.source_rows, [1]);
            assert_eq!(rows.cells, [Some("9007199254740993".into())]);
        }
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn filter_spec_keeps_zero_count_choices_across_ffi() {
        let source =
            std::env::temp_dir().join(format!("gewu-ffi-filter-spec-{}.csv", std::process::id()));
        std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let state = ViewState {
            sort: vec![],
            filters: vec![Filter::Equals {
                column: 1,
                value: "10".into(),
            }],
            search: None,
        };
        let FilterEditor::Choices { values } = doc.filter_spec(0, Some(state)).unwrap().editor
        else {
            panic!("small string columns should offer choices");
        };
        assert_eq!(values.len(), 2);
        let cat = values
            .iter()
            .find(|v| v.label.as_deref() == Some("cat"))
            .unwrap();
        let dog = values
            .iter()
            .find(|v| v.label.as_deref() == Some("dog"))
            .unwrap();
        assert_eq!(cat.count, 1);
        assert_eq!(dog.count, 0);
        let view = doc
            .apply_view(ViewState {
                sort: vec![],
                filters: vec![Filter::InValues {
                    column: 0,
                    keys: vec![dog.key],
                }],
                search: None,
            })
            .unwrap();
        assert_eq!(doc.rows(view.version, 0, 10, vec![0]).unwrap().cells, [
            Some("dog".into())
        ]);
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn histogram_bounds_cross_ffi_without_losing_large_ids() {
        use std::fmt::Write;

        let source =
            std::env::temp_dir().join(format!("gewu-ffi-range-spec-{}.csv", std::process::id()));
        let base = 9_007_199_254_740_993_i64;
        let mut csv = String::from("id\n");
        for offset in 0..1_000 {
            writeln!(csv, "{}", base + offset).unwrap();
        }
        std::fs::write(&source, csv).unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let FilterEditor::Range {
            distribution: Some(distribution),
            ..
        } = doc.filter_spec(0, None).unwrap().editor
        else {
            panic!("many distinct integers should offer a typed range histogram");
        };
        assert_eq!(distribution.min_label, base.to_string());
        assert_eq!(distribution.max_label, (base + 999).to_string());
        assert_eq!(
            distribution.ranges.len(),
            distribution.histogram.counts.len()
        );
        assert!(distribution.ranges.first().unwrap().min.is_none());
        assert!(distribution.ranges.last().unwrap().max.is_none());
        for bin in [
            0,
            distribution.ranges.len() / 2,
            distribution.ranges.len() - 1,
        ] {
            let bounds = &distribution.ranges[bin];
            let view = doc
                .apply_view(ViewState {
                    sort: vec![],
                    filters: vec![Filter::Range {
                        column: 0,
                        min: bounds.min.clone(),
                        max: bounds.max.clone(),
                    }],
                    search: None,
                })
                .unwrap();
            assert_eq!(view.row_count, distribution.histogram.counts[bin]);
        }
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn file_metadata_and_xlsx_export_cross_the_ffi_boundary() {
        let source =
            std::env::temp_dir().join(format!("gewu-ffi-formats-{}.csv", std::process::id()));
        std::fs::write(&source, "label,id\ncat,10\ndog,20\n").unwrap();
        let doc = GewuDocument::open(source.to_str().unwrap().to_owned()).unwrap();
        let info = doc.info();
        assert_eq!(info.format, "CSV");
        assert_eq!(info.row_count, 2);
        assert_eq!(info.row_groups, None);
        assert!(info.compression.is_empty());
        assert!(info.sheet.is_none());

        for (format, extension, name) in [
            (FileType::Xlsx, "xlsx", "XLSX"),
            (FileType::Parquet, "parquet", "Parquet"),
        ] {
            let destination = source.with_extension(extension);
            doc.prepare_export(
                doc.current_view().version,
                vec![0, 1],
                format,
                ExportOptions {
                    convert_nested_to_json: false,
                },
            )
            .unwrap()
            .write(destination.to_str().unwrap().to_owned())
            .unwrap();
            let reopened = GewuDocument::open(destination.to_str().unwrap().to_owned()).unwrap();
            let info = reopened.info();
            assert_eq!(info.format, name);
            assert_eq!(info.row_count, 2);
            if extension == "xlsx" {
                assert_eq!(info.row_groups, None);
                assert_eq!(info.sheets, vec![info.sheet.unwrap()]);
            } else {
                assert_eq!(info.row_groups, Some(1));
                assert!(!info.compression.is_empty());
                assert!(info.sheet.is_none());
            }
            std::fs::remove_file(destination).unwrap();
        }
        std::fs::remove_file(source).unwrap();
    }
}
