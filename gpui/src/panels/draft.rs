use std::collections::BTreeSet;

use gewu_core::{ColumnCapabilities, Filter, FilterCondition, TextMatch};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Mode {
    Values,
    Range,
    Text,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MissingPolicy {
    Exclude,
    Include,
    Only,
}

impl MissingPolicy {
    fn include(include: bool) -> Self {
        if include {
            Self::Include
        } else {
            Self::Exclude
        }
    }
}

/// Exact selections may contain values currently absent from the facet.
#[derive(Clone)]
pub(super) struct Draft {
    pub mode: Mode,
    pub missing: MissingPolicy,
    pub excluded: bool,
    pub values: BTreeSet<String>,
    pub comparison: TextMatch,
    pub case_sensitive: bool,
    pub search: String,
    pub low: String,
    pub high: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            mode: Mode::Missing,
            missing: MissingPolicy::Include,
            excluded: true,
            values: BTreeSet::new(),
            comparison: TextMatch::Contains,
            case_sensitive: false,
            search: String::new(),
            low: String::new(),
            high: String::new(),
        }
    }
}

impl Draft {
    pub fn from_filter(filter: Option<&Filter>) -> Self {
        let mut draft = Self::default();
        match filter {
            None => {}
            Some(Filter::OnlyNulls) => draft.missing = MissingPolicy::Only,
            Some(Filter::NotNull) => draft.missing = MissingPolicy::Exclude,
            Some(Filter::Condition {
                condition,
                include_nulls,
            }) => {
                draft.missing = MissingPolicy::include(*include_nulls);
                match condition {
                    FilterCondition::Exact { values, exclude } => {
                        draft.mode = Mode::Values;
                        draft.values.extend(values.iter().cloned());
                        draft.excluded = *exclude;
                    }
                    FilterCondition::Text {
                        text,
                        mode,
                        case_sensitive,
                    } => {
                        draft.mode = Mode::Text;
                        draft.search.clone_from(text);
                        draft.comparison = *mode;
                        draft.case_sensitive = *case_sensitive;
                    }
                    FilterCondition::Range { min, max } => {
                        draft.mode = Mode::Range;
                        draft.low = min.clone().unwrap_or_default();
                        draft.high = max.clone().unwrap_or_default();
                    }
                }
            }
        }
        draft
    }

    pub fn has_exact_condition(&self) -> bool {
        !self.excluded || !self.values.is_empty()
    }

    pub fn modes(cap: ColumnCapabilities, offer_values: bool) -> Vec<Mode> {
        let mut modes = Vec::new();
        if cap.exact && offer_values {
            modes.push(Mode::Values);
        }
        if cap.range {
            modes.push(Mode::Range);
        }
        if cap.text_match {
            modes.push(Mode::Text);
        }
        if modes.is_empty() {
            modes.push(Mode::Missing);
        }
        modes
    }

    pub fn value_checked(&self, value: &str) -> bool {
        self.missing != MissingPolicy::Only && (self.values.contains(value) != self.excluded)
    }

    pub fn toggle_include_missing(&mut self) {
        self.begin_value_edit();
        self.missing = MissingPolicy::include(self.missing == MissingPolicy::Exclude);
    }

    pub fn select_all(&mut self, selected: bool) {
        self.values.clear();
        self.excluded = selected;
        self.missing = MissingPolicy::include(selected);
    }

    /// The Only policy shows all present values unchecked. An edit starts from that selection.
    fn begin_value_edit(&mut self) {
        if self.missing == MissingPolicy::Only {
            self.values.clear();
            self.excluded = false;
            self.missing = MissingPolicy::Include;
        }
    }

    pub fn toggle(&mut self, value: String) {
        self.mode = Mode::Values;
        self.begin_value_edit();
        if !self.values.remove(&value) {
            self.values.insert(value);
        }
    }

    pub fn filter(&self) -> Option<Filter> {
        if self.missing == MissingPolicy::Only {
            return Some(Filter::OnlyNulls);
        }
        let condition = match self.mode {
            Mode::Values if self.has_exact_condition() => Some(FilterCondition::Exact {
                values: self.values.iter().cloned().collect(),
                exclude: self.excluded,
            }),
            Mode::Text if !self.search.is_empty() => Some(FilterCondition::Text {
                text: self.search.clone(),
                mode: self.comparison,
                case_sensitive: self.case_sensitive,
            }),
            Mode::Range if !self.low.trim().is_empty() || !self.high.trim().is_empty() => {
                let bound = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
                Some(FilterCondition::Range {
                    min: bound(&self.low),
                    max: bound(&self.high),
                })
            }
            _ => None,
        };
        condition
            .map(|condition| Filter::Condition {
                condition,
                include_nulls: self.missing == MissingPolicy::Include,
            })
            .or((self.missing == MissingPolicy::Exclude).then_some(Filter::NotNull))
    }
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
