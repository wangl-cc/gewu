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
    Contains {
        text: String,
        case_sensitive: bool,
    },
    Exact {
        values: Vec<String>,
        exclude: bool,
    },
}
impl From<Filter> for core::Filter {
    fn from(value: Filter) -> Self {
        Self {
            include_nulls: value.include_nulls,
            condition: match value.condition {
                FilterCondition::Range { min, max } => core::FilterCondition::Range { min, max },
                FilterCondition::Contains {
                    text,
                    case_sensitive,
                } => core::FilterCondition::Contains {
                    text,
                    case_sensitive,
                },
                FilterCondition::Exact { values, exclude } => {
                    core::FilterCondition::Exact { values, exclude }
                }
            },
        }
    }
}

impl From<core::Filter> for Filter {
    fn from(value: core::Filter) -> Self {
        Self {
            include_nulls: value.include_nulls,
            condition: match value.condition {
                core::FilterCondition::Range { min, max } => FilterCondition::Range { min, max },
                core::FilterCondition::Contains {
                    text,
                    case_sensitive,
                } => FilterCondition::Contains {
                    text,
                    case_sensitive,
                },
                core::FilterCondition::Exact { values, exclude } => {
                    FilterCondition::Exact { values, exclude }
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
impl From<core::Bound> for RangeBound {
    fn from(bound: core::Bound) -> Self {
        Self {
            value: bound.filter_text(),
            label: bound.display().into(),
            coordinate: bound.axis_value(),
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
