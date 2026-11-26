use std::{collections::BTreeSet, sync::Arc};

use gewu_core::{ColumnCapabilities, Filter, FilterChoices, FilterCondition, TextMatch};

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
pub(super) struct Draft {
    pub mode: Mode,
    pub missing: MissingPolicy,
    excluded: bool,
    values: BTreeSet<String>,
    facet: Option<Arc<FilterChoices>>,
    // Membership in `values` indexed by the current facet. Paint never compares full literals.
    listed: Vec<bool>,
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
            facet: None,
            listed: Vec::new(),
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

    pub fn facet(&self) -> Option<&Arc<FilterChoices>> {
        self.facet.as_ref()
    }

    pub fn set_facet(&mut self, facet: Option<Arc<FilterChoices>>) {
        self.listed = facet
            .as_ref()
            .and_then(|f| f.values.as_ref())
            .into_iter()
            .flatten()
            .map(|v| self.values.contains(&v.value))
            .collect();
        self.facet = facet;
    }

    pub fn exact_description(&self) -> String {
        super::exact_description(self.values.iter(), self.excluded)
    }

    pub fn value_checked(&self, index: usize) -> bool {
        self.missing != MissingPolicy::Only && (self.listed[index] != self.excluded)
    }

    pub fn reset(&mut self) {
        let facet = self.facet.take();
        *self = Self::default();
        self.set_facet(facet);
    }

    fn clear_values(&mut self) {
        self.values.clear();
        self.listed.fill(false);
    }

    pub fn toggle_include_missing(&mut self) {
        self.begin_value_edit();
        self.missing = MissingPolicy::include(self.missing == MissingPolicy::Exclude);
    }

    pub fn select_all(&mut self, selected: bool) {
        self.clear_values();
        self.excluded = selected;
        self.missing = MissingPolicy::include(selected);
    }

    /// The Only policy shows all present values unchecked. An edit starts from that selection.
    fn begin_value_edit(&mut self) {
        if self.missing == MissingPolicy::Only {
            self.clear_values();
            self.excluded = false;
            self.missing = MissingPolicy::Include;
        }
    }

    pub fn toggle(&mut self, source: &Arc<FilterChoices>, index: usize) {
        // A click from the previous facet can arrive after its rows have been replaced.
        let Some(choices) = source.values.as_ref().filter(|_| {
            self.facet()
                .is_some_and(|current| Arc::ptr_eq(current, source))
        }) else {
            return;
        };
        let value = &choices[index].value;
        self.mode = Mode::Values;
        self.begin_value_edit();
        self.listed[index] = if self.values.remove(value) {
            false
        } else {
            self.values.insert(value.clone())
        };
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
