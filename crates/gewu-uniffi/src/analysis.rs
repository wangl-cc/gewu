//! Concrete column-analysis results and inspector capability metadata.

use gewu_core::{
    AnalysisCapabilities, AnalysisField, AxisScale, Histogram, LengthSummary, LengthUnit,
    NumericStatistics, ValueFrequency, ValueSummary,
};

/// Type-level analysis support; the frontend owns its sections and their order.
#[uniffi::remote(Record)]
pub struct AnalysisCapabilities {
    pub statistics: bool,
    pub distribution: bool,
    pub values: bool,
    pub lengths: bool,
    pub structure: bool,
}

#[uniffi::remote(Enum)]
pub enum LengthUnit {
    Characters,
    Bytes,
    Elements,
}

#[uniffi::remote(Record)]
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

#[uniffi::remote(Record)]
pub struct ValueSummary {
    pub distinct_count: u64,
    pub values: Vec<ValueFrequency>,
}

#[uniffi::remote(Record)]
pub struct ValueFrequency {
    pub label: String,
    pub count: u64,
}

#[uniffi::remote(Record)]
pub struct LengthSummary {
    pub unit: LengthUnit,
    pub empty_count: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub mean: Option<f64>,
    pub histogram: Histogram,
}

#[uniffi::remote(Record)]
pub struct AnalysisField {
    pub name: String,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

/// Distribution of a numeric column with an approximate display axis.
/// Range filters use `RangeHistogram.ranges` for exact bin boundaries.
#[uniffi::remote(Record)]
pub struct Histogram {
    pub scale: AxisScale,
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
