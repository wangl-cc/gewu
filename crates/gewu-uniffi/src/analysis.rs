//! Concrete column-analysis results and inspector capability metadata.

use gewu_core as core;

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
    pub values: Vec<ValueFrequency>,
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
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

#[derive(uniffi::Record)]
pub struct ColumnOverview {
    pub rows: u64,
    pub null_count: u64,
}

impl From<core::ColumnOverview> for ColumnOverview {
    fn from(value: core::ColumnOverview) -> Self {
        Self {
            rows: value.rows,
            null_count: value.null_count,
        }
    }
}

impl From<core::NumericStatistics> for NumericStatistics {
    fn from(value: core::NumericStatistics) -> Self {
        Self {
            min_label: value.min_label,
            max_label: value.max_label,
            median_label: value.median_label,
            mean: value.mean,
            std_dev: value.std_dev,
            nan_count: value.nan_count,
            infinite_count: value.infinite_count,
        }
    }
}

impl From<core::ValueSummary> for ValueSummary {
    fn from(value: core::ValueSummary) -> Self {
        Self {
            distinct_count: value.distinct_count,
            estimated: value.estimated,
            values: value.values.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<core::LengthSummary> for LengthSummary {
    fn from(value: core::LengthSummary) -> Self {
        Self {
            unit: match value.unit {
                core::LengthUnit::Characters => LengthUnit::Characters,
                core::LengthUnit::Bytes => LengthUnit::Bytes,
                core::LengthUnit::Elements => LengthUnit::Elements,
            },
            empty_count: value.empty_count,
            min: value.min,
            max: value.max,
            mean: value.mean,
            histogram: value.histogram.into(),
        }
    }
}

impl From<core::AnalysisField> for AnalysisField {
    fn from(value: core::AnalysisField) -> Self {
        Self {
            name: value.name,
            display_type: value.display_type,
            technical_type: value.technical_type,
            nullable: value.nullable,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ValueFrequency {
    pub label: String,
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

impl From<core::ValueFrequency> for ValueFrequency {
    fn from(value: core::ValueFrequency) -> Self {
        Self {
            label: value.label,
            count: value.count,
        }
    }
}
