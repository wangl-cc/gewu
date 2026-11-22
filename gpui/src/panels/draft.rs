use std::collections::BTreeSet;

use gewu_core::{ColumnCapabilities, Filter, FilterCondition, TextMatch};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Mode {
    Values,
    Range,
    Text,
    Missing,
}

/// Exact selections may contain values currently absent from the facet.
#[derive(Clone)]
pub(super) struct Draft {
    pub mode: Mode,
    include_missing: bool,
    pub only_missing: bool,
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
            include_missing: true,
            only_missing: false,
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
            Some(Filter::OnlyNulls) => draft.only_missing = true,
            Some(Filter::NotNull) => draft.include_missing = false,
            Some(Filter::Condition {
                condition,
                include_nulls,
            }) => {
                draft.include_missing = *include_nulls;
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

    pub fn include_missing_checked(&self) -> bool {
        self.include_missing && !self.only_missing
    }

    pub fn toggle_include_missing(&mut self) {
        self.include_missing = !self.include_missing_checked();
        self.only_missing = false;
    }

    pub fn select_all(&mut self, selected: bool) {
        self.values.clear();
        self.excluded = selected;
        self.include_missing = selected;
        self.only_missing = false;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if mode == Mode::Values && self.only_missing {
            self.only_missing = false;
            if !self.has_exact_condition() {
                // Represent Only missing in the value list as an empty selection plus nulls.
                self.excluded = false;
                self.include_missing = true;
            }
        }
    }

    pub fn toggle(&mut self, value: String) {
        self.set_mode(Mode::Values);
        if !self.values.remove(&value) {
            self.values.insert(value);
        }
    }

    pub fn filter(&self) -> Option<Filter> {
        if self.only_missing {
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
                include_nulls: self.include_missing,
            })
            .or((!self.include_missing).then_some(Filter::NotNull))
    }
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
