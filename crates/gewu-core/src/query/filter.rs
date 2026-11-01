//! Declarative filter predicates and query planning costs.

use crate::Bound;

/// Declarative row-level filter predicate for a single column.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Equal to text parsed in this column's own type. Strings compare
    /// literally (case-sensitive); floats parse to their native width;
    /// decimal coefficients remain exact. Invalid input is an error.
    Equals { value: String },
    /// Match any input parsed with the same semantics as `Equals`, without a
    /// distinct-value index. Invalid input is an error; an empty list matches
    /// nothing. Nulls never match.
    EqualsAny { values: Vec<String> },
    /// Inclusive range with endpoints already parsed for the column's type.
    /// Unlike search, the number of typed decimal places does not widen
    /// these bounds. Missing ends are open; invalid or reversed bounds error.
    Range {
        min: Option<Bound>,
        max: Option<Bound>,
        /// Also match nulls in this column, without bypassing other filters.
        include_nulls: bool,
    },
    /// Keep rows whose value is one of `keys` (keys come from
    /// [`crate::Session::distinct_values`]). This is the Excel-style
    /// checkbox filter.
    InValues { keys: Vec<u64> },
    /// Exact, case-sensitive string values without a distinct-value index.
    /// `None` names null; an empty string remains a separate value. Exclusion
    /// represents all source values except the listed ones, including nulls.
    TextValues {
        values: Vec<Option<String>>,
        exclude: bool,
    },
    /// Keep rows where the value is null (or not null when `negate`).
    IsNull { negate: bool },
    /// Keep rows whose displayed text contains `text`. For text, lists and
    /// other non-ordered columns; numbers, dates, times and booleans are
    /// refused with [`crate::Error::InvalidArgument`] (use `Range` or
    /// `InValues`).
    Contains {
        text: String,
        case_sensitive: bool,
        /// Also match nulls in this column, without bypassing other filters.
        include_nulls: bool,
    },
}

impl Filter {
    /// Estimated evaluation cost (0 = cheapest, higher = more expensive).
    /// Used by the query planner to reorder AND predicates so that cheap,
    /// high-selectivity checks run first to trigger batch short-circuiting early.
    pub fn estimated_cost(&self) -> u8 {
        match self {
            // Null check: direct null bitmap test, near-zero cost.
            Filter::IsNull { .. } => 0,
            // InValues: bitmask or small integer code comparison against pre-indexed keys.
            Filter::InValues { .. } => 1,
            // TextValues: exact value-set selection (accelerated by ValueIndex when available).
            Filter::TextValues { .. } => 1,
            // Range: primitive numeric / temporal bounds comparison.
            Filter::Range { .. } => 2,
            // Equals: single-value typed equality.
            Filter::Equals { .. } => 3,
            // EqualsAny: set of typed equality checks.
            Filter::EqualsAny { .. } => 4,
            // Contains: string / list substring search.
            Filter::Contains { .. } => 5,
        }
    }
}
