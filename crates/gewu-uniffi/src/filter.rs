//! Text-only filter inputs and exact data projections for editors.
use gewu_core as core;

use crate::Histogram;

#[derive(uniffi::Record)]
pub struct Filter {
    pub condition: FilterCondition,
    pub include_nulls: bool,
}
#[derive(uniffi::Enum)]
pub enum FilterCondition {
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
    Exact {
        values: Vec<String>,
        exclude: bool,
    },
}
#[derive(uniffi::Enum, Clone, Copy)]
pub enum TextMatch {
    Contains,
    Equals,
}
impl From<TextMatch> for core::TextMatch {
    fn from(value: TextMatch) -> Self {
        match value {
            TextMatch::Contains => Self::Contains,
            TextMatch::Equals => Self::Equals,
        }
    }
}
impl From<Filter> for core::Filter {
    fn from(value: Filter) -> Self {
        Self {
            include_nulls: value.include_nulls,
            condition: match value.condition {
                FilterCondition::Range { min, max } => core::FilterCondition::Range { min, max },
                FilterCondition::Text {
                    text,
                    mode,
                    case_sensitive,
                } => core::FilterCondition::Text {
                    text,
                    mode: mode.into(),
                    case_sensitive,
                },
                FilterCondition::Exact { values, exclude } => {
                    core::FilterCondition::Exact { values, exclude }
                }
            },
        }
    }
}

/// Display and lossless wire text are separate; coordinates are only geometry.
#[derive(uniffi::Record)]
pub struct RangeBound {
    pub value: String,
    pub label: String,
    pub coordinate: f64,
}
impl From<core::RangeBound> for RangeBound {
    fn from(bound: core::RangeBound) -> Self {
        Self {
            value: bound.value,
            label: bound.label,
            coordinate: bound.coordinate,
        }
    }
}
#[derive(uniffi::Record)]
pub struct RangeBounds {
    pub min: Option<RangeBound>,
    pub max: Option<RangeBound>,
}
#[derive(uniffi::Record)]
pub struct RangeHistogram {
    pub histogram: Histogram,
    pub ranges: Vec<RangeBounds>,
}
impl From<core::RangeHistogram> for RangeHistogram {
    fn from(value: core::RangeHistogram) -> Self {
        Self {
            histogram: value.histogram.into(),
            ranges: value
                .ranges
                .into_iter()
                .map(|b| RangeBounds {
                    min: b.min.map(Into::into),
                    max: b.max.map(Into::into),
                })
                .collect(),
        }
    }
}
#[derive(uniffi::Record)]
pub struct ValueChoice {
    pub label: String,
    pub value: String,
    pub count: u64,
}
#[derive(uniffi::Record)]
pub struct FilterChoices {
    pub values: Option<Vec<ValueChoice>>,
    pub null_count: u64,
}
impl From<core::FilterChoices> for FilterChoices {
    fn from(value: core::FilterChoices) -> Self {
        Self {
            null_count: value.null_count,
            values: value.values.map(|values| {
                values
                    .into_iter()
                    .map(|v| ValueChoice {
                        label: v.label,
                        value: v.value,
                        count: v.count,
                    })
                    .collect()
            }),
        }
    }
}
