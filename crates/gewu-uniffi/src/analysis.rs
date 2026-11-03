//! Concrete column-analysis results and inspector capability metadata.

use gewu_core as core;

/// Type-level analysis support; the frontend owns its sections and their order.
#[derive(uniffi::Record)]
pub struct AnalysisCapabilities {
    pub statistics: bool,
    pub distribution: bool,
    pub values: bool,
    pub lengths: bool,
    pub structure: bool,
}

impl From<core::AnalysisCapabilities> for AnalysisCapabilities {
    fn from(value: core::AnalysisCapabilities) -> Self {
        Self {
            statistics: value.statistics,
            distribution: value.distribution,
            values: value.values,
            lengths: value.lengths,
            structure: value.structure,
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
    pub mean_label: Option<String>,
    pub std_dev: Option<f64>,
    pub nan_count: u64,
    pub infinite_count: u64,
}

#[derive(uniffi::Record)]
pub struct ValueSummary {
    pub distinct_count: u64,
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

impl From<core::NumericStatistics> for NumericStatistics {
    fn from(value: core::NumericStatistics) -> Self {
        Self {
            min_label: value.min_label,
            max_label: value.max_label,
            median_label: value.median_label,
            mean: value.mean,
            mean_label: value.mean_label,
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
    pub non_positive_count: u64,
    pub infinite_count: u64,
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
            non_positive_count: h.non_positive_count,
            infinite_count: h.infinite_count,
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
