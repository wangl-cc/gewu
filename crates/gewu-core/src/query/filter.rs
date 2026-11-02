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
    Contains {
        text: String,
        case_sensitive: bool,
    },
    Exact {
        values: Vec<String>,
        exclude: bool,
    },
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

    pub fn contains(text: String, case_sensitive: bool, include_nulls: bool) -> Self {
        Self {
            condition: FilterCondition::Contains {
                text,
                case_sensitive,
            },
            include_nulls,
        }
    }

    pub(crate) fn estimated_cost(&self) -> u8 {
        match &self.condition {
            FilterCondition::Exact { values, .. } if values.is_empty() => 0,
            FilterCondition::Exact { .. } => 1,
            FilterCondition::Range { .. } => 2,
            FilterCondition::Contains { .. } => 3,
        }
    }
}
