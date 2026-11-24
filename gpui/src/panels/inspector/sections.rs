use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Overview,
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
}

impl Section {
    pub(super) fn available(col: &ColumnInfo) -> impl Iterator<Item = Self> {
        let a = col.analysis;
        [
            (Self::Overview, true),
            (Self::Statistics, a.statistics),
            (Self::Distribution, a.distribution),
            (Self::Values, a.values),
            (Self::Lengths, a.lengths),
            (Self::Structure, a.structure),
        ]
        .into_iter()
        .filter_map(|(s, ok)| ok.then_some(s))
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Statistics => "Statistics",
            Self::Distribution => "Distribution",
            Self::Values => "Values",
            Self::Lengths => "Lengths",
            Self::Structure => "Structure",
        }
    }
}

impl InspectorSettings {
    pub(super) fn expanded(&self, section: Section, column: &ColumnInfo) -> bool {
        self.expansion[section as usize].unwrap_or(match section {
            Section::Overview => true,
            Section::Statistics | Section::Distribution => column.analysis.distribution,
            Section::Values => !column.analysis.distribution,
            Section::Lengths | Section::Structure => false,
        })
    }
}

pub(super) enum SectionData {
    Overview(u64),
    Statistics(Arc<NumericStatistics>),
    Distribution(Arc<RangeHistogram>),
    Values {
        distinct: u64,
        rows: Vec<(SharedString, u64)>,
    },
    Lengths(Arc<LengthSummary>),
    Structure(Vec<AnalysisField>),
}

pub(super) struct ColumnResults {
    pub(super) column: ColumnInfo,
    pub(super) sections: Vec<(Section, Option<gewu_core::Result<SectionData>>)>,
}

impl ColumnResults {
    pub(super) fn load(
        snapshot: &AppliedView,
        col: ColumnInfo,
        settings: InspectorSettings,
        scale: AxisScale,
    ) -> Self {
        let id = col.id;
        let cache = snapshot.analyses();
        let sections = Section::available(&col)
            .map(|section| {
                let result = settings.expanded(section, &col).then(|| match section {
                    Section::Overview => cache.count_nulls(id).map(SectionData::Overview),
                    Section::Statistics => cache.statistics(id).map(SectionData::Statistics),
                    Section::Distribution => cache
                        .distribution_with_ranges(id, settings.bins, scale)
                        .map(SectionData::Distribution),
                    Section::Values => cache.values(id).map(|s| SectionData::Values {
                        distinct: s.distinct_count,
                        rows: s
                            .values
                            .iter()
                            .map(|v| (super::super::value_preview(&v.label), v.count))
                            .collect(),
                    }),
                    Section::Lengths => cache.lengths(id, settings.bins).map(SectionData::Lengths),
                    Section::Structure => snapshot
                        .dataset()
                        .column_structure(id)
                        .map(SectionData::Structure),
                });
                (section, result)
            })
            .collect();
        Self {
            column: col,
            sections,
        }
    }
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
