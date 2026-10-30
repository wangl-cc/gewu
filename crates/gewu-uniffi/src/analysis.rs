//! Section-sized analysis records and independently cancellable jobs.

use gewu_core as core;

use crate::GewuError;

#[derive(uniffi::Enum, Clone, Copy)]
pub enum AnalysisSection {
    Overview,
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
}

impl From<core::AnalysisSection> for AnalysisSection {
    fn from(section: core::AnalysisSection) -> Self {
        match section {
            core::AnalysisSection::Overview => Self::Overview,
            core::AnalysisSection::Statistics => Self::Statistics,
            core::AnalysisSection::Distribution => Self::Distribution,
            core::AnalysisSection::Values => Self::Values,
            core::AnalysisSection::Lengths => Self::Lengths,
            core::AnalysisSection::Structure => Self::Structure,
        }
    }
}

impl From<AnalysisSection> for core::AnalysisSection {
    fn from(section: AnalysisSection) -> Self {
        match section {
            AnalysisSection::Overview => Self::Overview,
            AnalysisSection::Statistics => Self::Statistics,
            AnalysisSection::Distribution => Self::Distribution,
            AnalysisSection::Values => Self::Values,
            AnalysisSection::Lengths => Self::Lengths,
            AnalysisSection::Structure => Self::Structure,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum LengthUnit {
    Characters,
    Bytes,
    Elements,
}

#[derive(uniffi::Record)]
pub struct NumericStatistics {
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    pub median_label: Option<String>,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub nan_count: u64,
    pub infinite_count: u64,
}

#[derive(uniffi::Record)]
pub struct ValueSummary {
    pub distinct_count: u64,
    pub estimated: bool,
    pub values: Vec<DistinctValue>,
}

#[derive(uniffi::Record)]
pub struct LengthSummary {
    pub unit: LengthUnit,
    pub empty_count: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub mean: Option<f64>,
    pub histogram: Histogram,
}

#[derive(uniffi::Record)]
pub struct AnalysisField {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

#[derive(uniffi::Enum)]
pub enum AnalysisData {
    Overview { rows: u64, null_count: u64 },
    Statistics { statistics: NumericStatistics },
    Distribution { histogram: Histogram },
    Values { summary: ValueSummary },
    Lengths { summary: LengthSummary },
    Structure { fields: Vec<AnalysisField> },
}

#[derive(uniffi::Record)]
pub struct AnalysisResult {
    pub version: u64,
    pub column: u32,
    pub data: AnalysisData,
}

impl From<core::AnalysisResult> for AnalysisResult {
    fn from(result: core::AnalysisResult) -> Self {
        let data = match result.data {
            core::AnalysisData::Overview { rows, null_count } => {
                AnalysisData::Overview { rows, null_count }
            }
            core::AnalysisData::Statistics { statistics } => AnalysisData::Statistics {
                statistics: NumericStatistics {
                    min_label: statistics.min_label,
                    max_label: statistics.max_label,
                    median_label: statistics.median_label,
                    mean: statistics.mean,
                    std_dev: statistics.std_dev,
                    nan_count: statistics.nan_count,
                    infinite_count: statistics.infinite_count,
                },
            },
            core::AnalysisData::Distribution { histogram } => AnalysisData::Distribution {
                histogram: histogram.into(),
            },
            core::AnalysisData::Values { summary } => AnalysisData::Values {
                summary: ValueSummary {
                    distinct_count: summary.distinct_count,
                    estimated: summary.estimated,
                    values: summary.values.into_iter().map(Into::into).collect(),
                },
            },
            core::AnalysisData::Lengths { summary } => AnalysisData::Lengths {
                summary: LengthSummary {
                    unit: match summary.unit {
                        core::LengthUnit::Characters => LengthUnit::Characters,
                        core::LengthUnit::Bytes => LengthUnit::Bytes,
                        core::LengthUnit::Elements => LengthUnit::Elements,
                    },
                    empty_count: summary.empty_count,
                    min: summary.min,
                    max: summary.max,
                    mean: summary.mean,
                    histogram: summary.histogram.into(),
                },
            },
            core::AnalysisData::Structure { fields } => AnalysisData::Structure {
                fields: fields
                    .into_iter()
                    .map(|field| AnalysisField {
                        name: field.name,
                        type_name: field.type_name,
                        nullable: field.nullable,
                    })
                    .collect(),
            },
        };
        Self {
            version: result.version,
            column: result.column,
            data,
        }
    }
}

/// A captured column/view section with independent, terminal cancellation.
#[derive(uniffi::Object)]
pub struct ColumnAnalysis {
    pub(crate) inner: core::ColumnAnalysis,
}

#[uniffi::export]
impl ColumnAnalysis {
    /// Slow: execute off the UI thread. Returned metadata identifies the snapshot.
    pub fn run(&self) -> Result<AnalysisResult, GewuError> {
        Ok(self.inner.run()?.into())
    }

    /// Thread-safe; does not cancel table queries or other analysis jobs.
    pub fn cancel(&self) {
        self.inner.cancel();
    }
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
pub struct DistinctValue {
    pub key: u64,
    pub label: Option<String>,
    pub count: u64,
}

/// Distribution of a numeric column with an approximate display axis.
/// Range filters use `RangeHistogram.ranges` for exact bin boundaries.
#[derive(uniffi::Record)]
pub struct Histogram {
    pub scale: crate::AxisScale,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    pub integer: bool,
    pub counts: Vec<u64>,
    pub bin_edges: Vec<f64>,
    pub null_count: u64,
    pub nan_count: u64,
    pub non_positive_count: u64,
    pub infinite_count: u64,
    pub total: u64,
}

impl From<core::Histogram> for Histogram {
    fn from(h: core::Histogram) -> Self {
        Self {
            scale: h.scale.into(),
            lo: h.lo,
            hi: h.hi,
            min_label: h.min_label,
            max_label: h.max_label,
            integer: h.integer,
            counts: h.counts,
            bin_edges: h.bin_edges,
            null_count: h.null_count,
            nan_count: h.nan_count,
            non_positive_count: h.non_positive_count,
            infinite_count: h.infinite_count,
            total: h.total,
        }
    }
}

impl From<core::DistinctValue> for DistinctValue {
    fn from(value: core::DistinctValue) -> Self {
        Self {
            key: value.key,
            label: value.label,
            count: value.count,
        }
    }
}

impl From<core::ColumnSummary> for ColumnSummary {
    fn from(summary: core::ColumnSummary) -> Self {
        Self {
            rows: summary.rows,
            null_count: summary.null_count,
            nan_count: summary.nan_count,
            distinct_count: summary.distinct_count,
            min: summary.min,
            max: summary.max,
            min_label: summary.min_label,
            max_label: summary.max_label,
            mean: summary.mean,
            std_dev: summary.std_dev,
            top_values: summary.top_values.into_iter().map(Into::into).collect(),
        }
    }
}
