//! Filter inputs, editor data and facet queries.

use std::collections::HashSet;

use arrow::array::Array;

use super::Session;
use crate::{
    column::{dictionary::NULL_CODE, stats},
    format,
    predicate::Predicate,
    scope::RowScope,
    types::{Domain, LogicalType},
    Error, Filter, Histogram, QuerySpec, RangeHistogram, Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Stable for the lifetime of the document; pass it to [`crate::Filter::InValues`].
    /// Codes `0..=254` name non-null values and `255` names null.
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
    /// Whether the keys name values for [`crate::Filter::InValues`]. False for
    /// columns with too many distinct values to encode; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

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
        let dictionary = self.dictionary(column)?;

        // Dictionary-encoded text keeps its choices editor rather than the
        // text editor; encoding and value semantics are separate here.
        if col.ty().direct_domain() == Some(Domain::Text) {
            return Ok(FilterSpec {
                nullable: col.is_nullable(),
                editor: FilterEditor::Text {
                    modes: vec![
                        crate::TextFilterMode::Choices,
                        crate::TextFilterMode::Contains,
                    ],
                    recommended: if dictionary.is_some() {
                        crate::TextFilterMode::Choices
                    } else {
                        crate::TextFilterMode::Contains
                    },
                },
            });
        }
        let editor = if let Some(dictionary) = &dictionary {
            let rows = self.rows_in(RowScope::facet(spec, column))?;
            let counts = dictionary.tally(&rows);
            FilterEditor::Choices {
                values: format_choices(
                    dictionary.values().as_ref(),
                    dictionary.has_null(),
                    &counts,
                    true,
                )?,
            }
        } else if col.capabilities().range {
            let rows = self.rows_in(RowScope::facet(spec, column))?;
            FilterEditor::Range {
                logical_type: col.logical_type().clone(),
                distribution: stats::build_scaled(&col.array, &rows, bins, scale)?,
            }
        } else if col.capabilities().contains {
            FilterEditor::Contains
        } else {
            FilterEditor::NullOnly
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
        let is_text = col.ty().domain() == Some(Domain::Text);
        if !is_text {
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
    /// A column with at most 255 non-null distinct values is encoded: the
    /// result is the first `limit` values in value order, and their keys work
    /// in [`Filter::InValues`]. A column with more is counted without an
    /// encoding: the result is the `limit` most common values (still in value
    /// order), `keyed` is false and the keys only number them.
    pub fn distinct_values(
        &self,
        column: u32,
        spec: Option<&QuerySpec>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let col = self.column(column)?;
        let dictionary = self.dictionary(column)?;
        let rows = self.rows_in(RowScope::facet(spec, column))?;
        let limit = limit as usize;
        match dictionary {
            Some(dictionary) => {
                let counts = dictionary.tally(&rows);
                let choices = format_choices(
                    dictionary.values().as_ref(),
                    dictionary.has_null(),
                    &counts,
                    false,
                )?;
                let total = choices.len() as u64;
                Ok(DistinctResult {
                    total,
                    truncated: total > limit as u64,
                    values: choices.into_iter().take(limit).collect(),
                    keyed: true,
                })
            }
            None => {
                let counted =
                    crate::column::dictionary::streaming::count_values(&col.array, &rows, limit)?;
                Ok(DistinctResult {
                    total: counted.distinct,
                    truncated: counted.distinct > limit as u64,
                    values: counted.top.into_iter().take(limit).collect(),
                    keyed: false,
                })
            }
        }
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
        let dictionary = self.keyed_dictionary(column)?;
        let target: HashSet<&Option<String>> = labels.iter().collect();
        let mut keys = Vec::new();
        if dictionary.has_null() && labels.iter().any(Option::is_none) {
            keys.push(u64::from(NULL_CODE));
        }
        let formatter = format::formatter(dictionary.values().as_ref())?;
        for code in 0..dictionary.len() {
            let label = format::cell(dictionary.values().as_ref(), &formatter, code);
            if target.contains(&label) {
                keys.push(code as u64);
            }
        }
        Ok(keys)
    }

    pub(super) fn prepare_filter(&self, column_idx: u32, filter: &Filter) -> Result<Predicate<'_>> {
        let col = self.column(column_idx)?;
        match filter {
            Filter::InValues { keys } => {
                Ok(self.keyed_dictionary(column_idx)?.prepare_predicate(keys))
            }
            Filter::TextValues { values, exclude } => {
                if col.ty().direct_domain() != Some(Domain::Text) {
                    return Err(Error::InvalidArgument(format!(
                        "{} is not a string column",
                        col.array.data_type()
                    )));
                }
                if let Some(dictionary) = self.dictionary(column_idx)? {
                    let keys = self.keys_for_labels(column_idx, values)?;
                    let mut allowed = [*exclude; 256];
                    for key in keys {
                        if let Some(slot) = allowed.get_mut(key as usize) {
                            *slot = !exclude;
                        }
                    }
                    return Ok(dictionary.predicate_from_allowed(allowed));
                }
                col.prepare_filter(filter)
            }
            _ => col.prepare_filter(filter),
        }
    }
}

pub(crate) fn format_choices(
    values: &dyn Array,
    has_null: bool,
    counts: &[u64; 256],
    include_zero: bool,
) -> Result<Vec<DistinctValue>> {
    let formatter = format::formatter(values)?;
    let mut result = Vec::with_capacity(values.len() + 1);
    let null_count = counts[NULL_CODE as usize];
    if has_null && (include_zero || null_count > 0) {
        result.push(DistinctValue {
            key: u64::from(NULL_CODE),
            label: None,
            count: null_count,
        });
    }
    for (code, &count) in counts.iter().enumerate().take(values.len()) {
        if include_zero || count > 0 {
            result.push(DistinctValue {
                key: code as u64,
                label: format::cell(values, &formatter, code),
                count,
            });
        }
    }
    Ok(result)
}
