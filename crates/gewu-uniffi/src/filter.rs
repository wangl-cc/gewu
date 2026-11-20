//! Text-only filter inputs and exact data projections for editors.
use gewu_core::{
    Filter, FilterChoices, FilterCondition, Histogram, RangeBound, RangeBounds, RangeHistogram,
    TextMatch, ValueChoice,
};

/// Which rows of one column pass.
#[uniffi::remote(Enum)]
pub enum Filter {
    /// Rows whose value is null.
    OnlyNulls,
    /// Rows whose value is not null.
    NotNull,
    /// Rows whose value satisfies `condition`, and null rows when `include_nulls`.
    Condition {
        condition: FilterCondition,
        include_nulls: bool,
    },
}

#[uniffi::remote(Enum)]
pub enum FilterCondition {
    /// Inclusive bounds as exact value text; a missing bound is open.
    Range {
        min: Option<String>,
        max: Option<String>,
    },
    /// Match text values. Case-insensitive matching uses Unicode simple case folding.
    Text {
        text: String,
        mode: TextMatch,
        case_sensitive: bool,
    },
    /// Select, or with `exclude` reject, the listed values.
    Exact { values: Vec<String>, exclude: bool },
}

#[uniffi::remote(Enum)]
pub enum TextMatch {
    Contains,
    Equals,
}

/// Display and lossless wire text are separate; coordinates are only geometry.
#[uniffi::remote(Record)]
pub struct RangeBound {
    pub value: String,
    pub label: String,
    pub coordinate: f64,
}

#[uniffi::remote(Record)]
pub struct RangeBounds {
    pub min: Option<RangeBound>,
    pub max: Option<RangeBound>,
}

#[uniffi::remote(Record)]
pub struct RangeHistogram {
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
}

#[uniffi::remote(Record)]
pub struct ValueChoice {
    pub label: String,
    pub value: String,
    pub count: u64,
}

#[uniffi::remote(Record)]
pub struct FilterChoices {
    pub values: Option<Vec<ValueChoice>>,
    pub null_count: u64,
}
