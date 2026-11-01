//! Filter inputs, editor data and facet queries.

use super::Session;
use crate::{
    column::{index::count_values, stats, DistinctValue, FilterEditorKind, LogicalType},
    predicate::Predicate,
    scope::RowScope,
    DistinctResult, Error, Filter, Histogram, QuerySpec, RangeHistogram, Result,
};

/// The editor and data a frontend needs for one column's filter popover.
/// The editor is chosen from the whole source column and stays stable as
/// other filters change. Counts exclude this column's own filters.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterSpec {
    pub nullable: bool,
    pub editor: FilterEditor,
}

#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "FilterEditor is created on demand for UI filter popovers and never stored in collections"
)]
pub enum FilterEditor {
    /// String filters always offer both modes. Source cardinality only selects
    /// the initial recommendation; candidate pages are requested separately.
    Text {
        modes: Vec<TextFilterMode>,
        recommended: TextFilterMode,
    },
    Range {
        logical_type: LogicalType,
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

impl Session {
    /// Parse text once at the input boundary, retaining the column's full type.
    pub fn parse_bound(&self, column: u32, text: &str) -> Result<crate::Bound> {
        self.column(column)?.parse_bound(text)
    }

    pub fn make_bound_i64(&self, column: u32, value: i64) -> Result<crate::Bound> {
        self.column(column)?.make_bound_i64(value)
    }

    pub fn make_bound_u64(&self, column: u32, value: u64) -> Result<crate::Bound> {
        self.column(column)?.make_bound_u64(value)
    }

    pub fn make_bound_f64(&self, column: u32, value: f64) -> Result<crate::Bound> {
        self.column(column)?.make_bound_f64(value)
    }

    /// Decimal coefficient is transported as text to accommodate 256-bit values.
    pub fn make_bound_decimal(
        &self,
        column: u32,
        coefficient: &str,
        scale: i8,
    ) -> Result<crate::Bound> {
        self.column(column)?.make_bound_decimal(coefficient, scale)
    }

    /// Describe the filter editor using whole-column cardinality. Facet counts
    /// respect other columns' explicit filters without changing the editor.
    pub fn filter_spec(&self, column: u32, spec: Option<&QuerySpec>) -> Result<FilterSpec> {
        self.filter_spec_with_bins(column, spec, 60)
    }

    /// Like filter_spec, with a requested histogram resolution (1 through 60).
    pub fn filter_spec_with_bins(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
    ) -> Result<FilterSpec> {
        self.filter_spec_with_scale(column, spec, bins, crate::AxisScale::Linear)
    }

    /// Like `filter_spec_with_bins`, with an explicit numeric binning scale.
    pub fn filter_spec_with_scale(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
        scale: crate::AxisScale,
    ) -> Result<FilterSpec> {
        if !(1..=60).contains(&bins) {
            return Err(Error::InvalidArgument(
                "filter histogram bins must be between 1 and 60".into(),
            ));
        }
        let col = self.column(column)?;
        if scale == crate::AxisScale::Log10 && !col.capabilities().logarithmic {
            return Err(Error::InvalidArgument(
                "logarithmic binning requires a numeric column".into(),
            ));
        }
        let editor_kind = self.dataset.filter_editor_kind(column)?;
        if *col.logical_type() == LogicalType::Text {
            return Ok(FilterSpec {
                nullable: col.is_nullable(),
                editor: FilterEditor::Text {
                    modes: vec![
                        crate::TextFilterMode::Choices,
                        crate::TextFilterMode::Contains,
                    ],
                    recommended: if matches!(editor_kind, FilterEditorKind::Choices) {
                        crate::TextFilterMode::Choices
                    } else {
                        crate::TextFilterMode::Contains
                    },
                },
            });
        }
        let editor = match editor_kind {
            FilterEditorKind::Choices => {
                let rows = self.rows_in(RowScope::facet(spec, column))?;
                FilterEditor::Choices {
                    values: self.keyed_index(column)?.counts_all(&rows),
                }
            }
            FilterEditorKind::Range => {
                let rows = self.rows_in(RowScope::facet(spec, column))?;
                FilterEditor::Range {
                    logical_type: col.logical_type().clone(),
                    distribution: stats::build_scaled(&col.array, &rows, bins, scale)?,
                }
            }
            FilterEditorKind::Contains => FilterEditor::Contains,
            FilterEditorKind::NullOnly => FilterEditor::NullOnly,
        };
        Ok(FilterSpec {
            nullable: col.is_nullable(),
            editor,
        })
    }

    /// Capture a string column and facet rows for bounded candidate paging.
    /// Preparation evaluates the other filters and must run off the UI thread.
    /// The returned session has independent cancellation for its page scans;
    /// preparation itself completes before that cancellation handle is returned.
    pub fn prepare_text_choices(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
    ) -> Result<crate::TextChoices> {
        let col = self.column(column)?;
        if *col.logical_type() != LogicalType::Text {
            return Err(Error::InvalidArgument(
                "text choices require a string column".into(),
            ));
        }
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        crate::TextChoices::new(col.array.clone(), rows)
    }

    /// Distinct values of a column with their counts, for the checkbox filter.
    ///
    /// With `spec`, counts only rows that pass the filters on *other*
    /// columns (like Excel), and leaves out values with no such rows.
    ///
    /// A column with at most 100,000 distinct values is indexed: the result
    /// is the first `limit` values in value order, and their keys work in
    /// [`Filter::InValues`]. A column with more is counted without an index:
    /// the result is the `limit` most common values (still in value order),
    /// `keyed` is false and the keys only number them.
    pub fn distinct_values(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let index = self.index(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        let limit = limit as usize;
        Ok(match index {
            Some(idx) => {
                let present = idx.counts(&rows);
                DistinctResult {
                    total: present.len() as u64,
                    truncated: present.len() > limit,
                    values: present.into_iter().take(limit).collect(),
                    keyed: true,
                }
            }
            None => {
                let col = self.column(column)?;
                let counted = count_values(&col.array, &rows, limit)?;
                DistinctResult {
                    total: counted.distinct,
                    truncated: counted.distinct > limit as u64,
                    values: counted.top,
                    keyed: false,
                }
            }
        })
    }

    /// Distribution of a numeric, date or time column in `bins` bins, for
    /// the range filter. With `spec`, counts only rows that pass the
    /// filters on *other* columns, like [`Self::distinct_values`].
    pub fn histogram(&self, column: u32, spec: Option<&QuerySpec>, bins: u32) -> Result<Histogram> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        stats::counts(&col.array, &rows, bins)
    }

    /// Distribution with exact inclusive bounds for each bin. Counts use the
    /// same facet scope as [`Self::histogram`]; unbounded outer edges include
    /// infinities. Empty/all-NaN data returns no ranges. Unlike filter editor
    /// charts, this preserves the requested bin count.
    pub fn histogram_ranges(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        bins: u32,
    ) -> Result<crate::RangeHistogram> {
        let col = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        stats::ranges(&col.array, &rows, bins)
    }

    /// Validate and prepare an explicit filter without scanning data. Value-list
    /// filters may build the column's lazy index; other filters only parse scalars.
    pub fn validate_filter(&self, column: u32, filter: &Filter) -> Result<()> {
        self.prepare_filter(column, filter).map(|_| ())
    }

    /// Keys for the given displayed labels (`None` = null). Unknown labels
    /// are ignored. Used when filtering by displayed value text.
    pub fn keys_for_labels(&self, column: u32, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let idx = self.keyed_index(column)?;
        Ok(idx.keys_for_labels(labels))
    }

    pub(super) fn prepare_filter(&self, column_idx: u32, filter: &Filter) -> Result<Predicate<'_>> {
        let col = self.column(column_idx)?;
        match filter {
            Filter::InValues { keys } => {
                Ok(col.prepare_values(self.keyed_index(column_idx)?, keys))
            }
            Filter::TextValues { values, exclude } => {
                if *col.logical_type() != LogicalType::Text {
                    return Err(Error::InvalidArgument(format!(
                        "{} is not a string column",
                        col.array.data_type()
                    )));
                }
                if let Some(index) = self.index(column_idx)? {
                    return Ok(index.prepare_text_predicate(values, *exclude));
                }
                col.prepare_filter(filter)
            }
            _ => col.prepare_filter(filter),
        }
    }
}
