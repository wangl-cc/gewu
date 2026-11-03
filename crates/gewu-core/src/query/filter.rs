//! Text filter protocol. Parsing and physical execution belong to the column.

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub condition: FilterCondition,
    /// Null handling is independent of value matching and exclusion.
    pub include_nulls: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterCondition {
    Range {
        min: Option<String>,
        max: Option<String>,
    },
    /// Match text values against `text`. Case-insensitive matching uses Unicode
    /// simple case folding for every mode.
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

/// How a text filter compares each value with its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMatch {
    /// The value contains the input as a substring.
    Contains,
    /// The whole value equals the input.
    Equals,
}

impl Filter {
    pub fn exact(values: Vec<String>, exclude: bool, include_nulls: bool) -> Self {
        Self {
            condition: FilterCondition::Exact { values, exclude },
            include_nulls,
        }
    }

    pub fn range(min: Option<String>, max: Option<String>, include_nulls: bool) -> Self {
        Self {
            condition: FilterCondition::Range { min, max },
            include_nulls,
        }
    }

    pub fn text(text: String, mode: TextMatch, case_sensitive: bool, include_nulls: bool) -> Self {
        Self {
            condition: FilterCondition::Text {
                text,
                mode,
                case_sensitive,
            },
            include_nulls,
        }
    }

    pub fn contains(text: String, case_sensitive: bool, include_nulls: bool) -> Self {
        Self::text(text, TextMatch::Contains, case_sensitive, include_nulls)
    }
}
