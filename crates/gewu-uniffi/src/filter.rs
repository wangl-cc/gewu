//! Typed filters, bounds and editor descriptions.

use std::sync::Arc;

use gewu_core as core;

use crate::{DistinctValue, Histogram, LogicalType};

#[derive(uniffi::Enum)]
pub enum Filter {
    TextValues {
        column: u32,
        values: Vec<Option<String>>,
        exclude: bool,
    },
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
        include_nulls: bool,
    },
    /// Exact typed equality; parsing is owned by the column's type.
    Equals {
        column: u32,
        value: String,
    },
    /// Any exact typed equality, without a distinct-value index.
    EqualsAny {
        column: u32,
        values: Vec<String>,
    },
    /// Inclusive typed bounds. A missing endpoint is open.
    Range {
        column: u32,
        min: Option<Arc<Bound>>,
        max: Option<Arc<Bound>>,
        include_nulls: bool,
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
}

#[derive(uniffi::Enum)]
pub enum TextFilterMode {
    Choices,
    Contains,
}

impl From<core::TextFilterMode> for TextFilterMode {
    fn from(mode: core::TextFilterMode) -> Self {
        match mode {
            core::TextFilterMode::Choices => Self::Choices,
            core::TextFilterMode::Contains => Self::Contains,
        }
    }
}

#[derive(uniffi::Enum)]
#[expect(
    clippy::large_enum_variant,
    reason = "FilterEditor is created on demand for UI filter popovers and never stored in collections"
)]
pub enum FilterEditor {
    Text {
        modes: Vec<TextFilterMode>,
        recommended: TextFilterMode,
    },
    Range {
        logical_type: LogicalType,
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
            core::FilterEditor::Text { modes, recommended } => FilterEditor::Text {
                modes: modes.into_iter().map(Into::into).collect(),
                recommended: recommended.into(),
            },
            core::FilterEditor::Range {
                logical_type,
                distribution,
            } => FilterEditor::Range {
                logical_type: logical_type.into(),
                distribution: distribution.map(|distribution| RangeHistogram {
                    histogram: distribution.histogram.into(),
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
                values: values.into_iter().map(Into::into).collect(),
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

impl From<Filter> for core::Filter {
    fn from(filter: Filter) -> Self {
        match filter {
            Filter::TextValues {
                column,
                values,
                exclude,
            } => Self::TextValues {
                column,
                values,
                exclude,
            },
            Filter::InValues { column, keys } => Self::InValues { column, keys },
            Filter::IsNull { column, negate } => Self::IsNull { column, negate },
            Filter::Contains {
                column,
                text,
                case_sensitive,
                include_nulls,
            } => Self::Contains {
                column,
                text,
                case_sensitive,
                include_nulls,
            },
            Filter::Equals { column, value } => Self::Equals { column, value },
            Filter::EqualsAny { column, values } => Self::EqualsAny { column, values },
            Filter::Range {
                column,
                min,
                max,
                include_nulls,
            } => Self::Range {
                column,
                min: min.map(|bound| bound.inner.clone()),
                max: max.map(|bound| bound.inner.clone()),
                include_nulls,
            },
        }
    }
}
