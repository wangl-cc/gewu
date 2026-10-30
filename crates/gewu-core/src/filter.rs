//! Explicit filter inputs and frontend editor descriptions.

use crate::{DistinctValue, RangeHistogram};

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Equal to text parsed in this column's own type. Strings compare
    /// literally (case-sensitive); floats parse to their native width;
    /// decimal coefficients remain exact. Invalid input is an error.
    Equals { column: u32, value: String },
    /// Match any input parsed with the same semantics as `Equals`, without a
    /// distinct-value index. Invalid input is an error; an empty list matches
    /// nothing. Nulls never match.
    EqualsAny { column: u32, values: Vec<String> },
    /// Inclusive range with endpoints already parsed for the column's type.
    /// Unlike search, the number of typed decimal places does not widen
    /// these bounds. Missing ends are open; invalid or reversed bounds error.
    Range {
        column: u32,
        min: Option<crate::Bound>,
        max: Option<crate::Bound>,
        /// Also match nulls in this column, without bypassing other filters.
        include_nulls: bool,
    },
    /// Keep rows whose value is one of `keys` (keys come from
    /// [`crate::Document::distinct_values`]). This is the Excel-style
    /// checkbox filter.
    InValues { column: u32, keys: Vec<u64> },
    /// Exact, case-sensitive string values without a distinct-value index.
    /// `None` names null; an empty string remains a separate value. Exclusion
    /// represents all source values except the listed ones, including nulls.
    TextValues {
        column: u32,
        values: Vec<Option<String>>,
        exclude: bool,
    },
    /// Keep rows where the value is null (or not null when `negate`).
    IsNull { column: u32, negate: bool },
    /// Keep rows whose displayed text contains `text`. For text, lists and
    /// other non-ordered columns; numbers, dates, times and booleans are
    /// refused with [`crate::Error::InvalidArgument`] (use `Range` or
    /// `InValues`).
    Contains {
        column: u32,
        text: String,
        case_sensitive: bool,
        /// Also match nulls in this column, without bypassing other filters.
        include_nulls: bool,
    },
}

impl Filter {
    pub fn column(&self) -> u32 {
        match self {
            Filter::InValues { column, .. }
            | Filter::TextValues { column, .. }
            | Filter::IsNull { column, .. }
            | Filter::Contains { column, .. }
            | Filter::Equals { column, .. }
            | Filter::EqualsAny { column, .. }
            | Filter::Range { column, .. } => *column,
        }
    }
}

/// The editor and data a frontend needs for one column's filter.
/// The editor is chosen from the whole source column and stays stable as
/// other filters change. Counts exclude this column's own filters.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterSpec {
    pub nullable: bool,
    pub editor: FilterEditor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterEditor {
    /// String filters always offer both modes. Source cardinality only selects
    /// the initial recommendation; candidate pages are requested separately.
    Text {
        modes: Vec<TextFilterMode>,
        recommended: TextFilterMode,
    },
    Range {
        kind: crate::DataKind,
        distribution: Option<RangeHistogram>,
    },
    /// All source values, including values with zero rows in the current facet.
    Choices {
        values: Vec<DistinctValue>,
    },
    Contains,
    NullOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFilterMode {
    Choices,
    Contains,
}
