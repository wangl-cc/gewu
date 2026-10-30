//! Filter inputs, editor data and facet queries.

use super::{Document, Indexed};
use crate::{
    column::{histogram, Column, FilterEditorKind, CHOICE_LIMIT},
    predicate::Predicate,
    scope::RowScope,
    value_index::count_values,
    DataKind, DistinctResult, Error, Filter, Histogram, Result, ViewState,
};

impl Document {
    /// Parse text once at the input boundary, retaining the column's full type.
    pub fn parse_bound(&self, column: u32, text: &str) -> Result<crate::Bound> {
        crate::Bound::parse(Column::new(self.column(column)?).logical_data_type(), text)
    }

    pub fn make_bound_i64(&self, column: u32, value: i64) -> Result<crate::Bound> {
        crate::Bound::from_i64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    pub fn make_bound_u64(&self, column: u32, value: u64) -> Result<crate::Bound> {
        crate::Bound::from_u64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    pub fn make_bound_f64(&self, column: u32, value: f64) -> Result<crate::Bound> {
        crate::Bound::from_f64(Column::new(self.column(column)?).logical_data_type(), value)
    }

    /// Decimal coefficient is transported as text to accommodate 256-bit values.
    pub fn make_bound_decimal(
        &self,
        column: u32,
        coefficient: &str,
        scale: i8,
    ) -> Result<crate::Bound> {
        let coefficient = coefficient
            .parse::<arrow::datatypes::i256>()
            .map_err(|_| Error::InvalidArgument("invalid decimal coefficient".into()))?;
        crate::Bound::from_decimal(
            Column::new(self.column(column)?).logical_data_type(),
            coefficient,
            scale,
        )
    }

    /// Describe the filter editor using whole-column cardinality. Facet counts
    /// respect other columns' filters and search without changing the editor.
    pub fn filter_spec(&self, column: u32, state: Option<&ViewState>) -> Result<crate::FilterSpec> {
        self.filter_spec_with_bins(column, state, 60)
    }

    /// Like filter_spec, with a requested histogram resolution (1 through 60).
    pub fn filter_spec_with_bins(
        &self,
        column: u32,
        state: Option<&ViewState>,
        bins: u32,
    ) -> Result<crate::FilterSpec> {
        if !(1..=60).contains(&bins) {
            return Err(Error::InvalidArgument(
                "filter histogram bins must be between 1 and 60".into(),
            ));
        }
        use crate::{FilterEditor, FilterSpec};
        let array = self.column(column)?;
        let typed = Column::new(array);
        let cell = &self.filter_editors[column as usize];
        let editor_kind = match cell.get() {
            Some(kind) => *kind,
            None => {
                let supported =
                    arrow::row::RowConverter::supports_fields(&[arrow::row::SortField::new(
                        array.data_type().clone(),
                    )]);
                let low_cardinality = if !supported {
                    false
                } else {
                    match self.indexes[column as usize].get() {
                        Some(Indexed::Built(index)) => index.len() <= CHOICE_LIMIT,
                        Some(Indexed::TooMany) => false,
                        None => crate::value_index::has_at_most(array, CHOICE_LIMIT)?,
                    }
                };
                *cell.get_or_init(|| typed.filter_editor(low_cardinality))
            }
        };
        if DataKind::of(typed.logical_data_type()) == DataKind::Text {
            return Ok(FilterSpec {
                nullable: self.info.columns[column as usize].nullable,
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
                let rows = self.rows_in(RowScope::facet(state, column))?;
                FilterEditor::Choices {
                    values: self.keyed_index(column)?.counts_all(&rows),
                }
            }
            FilterEditorKind::Range => {
                let rows = self.rows_in(RowScope::facet(state, column))?;
                FilterEditor::Range {
                    kind: DataKind::of(typed.logical_data_type()),
                    distribution: histogram::build(array, &rows, bins)?,
                }
            }
            FilterEditorKind::Contains => FilterEditor::Contains,
            FilterEditorKind::NullOnly => FilterEditor::NullOnly,
        };
        Ok(FilterSpec {
            nullable: self.info.columns[column as usize].nullable,
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
        state: Option<&ViewState>,
    ) -> Result<crate::TextChoices> {
        let array = self.column(column)?;
        if DataKind::of(array.data_type()) != DataKind::Text {
            return Err(Error::InvalidArgument(
                "text choices require a string column".into(),
            ));
        }
        let rows = self.rows_in(RowScope::facet(state, column))?;
        crate::TextChoices::new(array.clone(), rows)
    }

    /// Distinct values of a column with their counts, for the checkbox filter.
    ///
    /// With `filters`, counts only rows that pass the filters on *other*
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
        filters: Option<&ViewState>,
        limit: u32,
    ) -> Result<DistinctResult> {
        let index = self.index(column)?;
        let rows = self.rows_in(RowScope::facet(filters, column))?;
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
                let counted = count_values(self.column(column)?, &rows, limit)?;
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
    /// the range filter. With `filters`, counts only rows that pass the
    /// filters on *other* columns, like [`Self::distinct_values`].
    pub fn histogram(
        &self,
        column: u32,
        filters: Option<&ViewState>,
        bins: u32,
    ) -> Result<Histogram> {
        let array = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(filters, column))?;
        histogram::counts(array, &rows, bins)
    }

    /// Distribution with exact inclusive bounds for each bin. Counts use the
    /// same facet scope as [`Self::histogram`]; unbounded outer edges include
    /// infinities. Empty/all-NaN data returns no ranges. Unlike filter editor
    /// charts, this preserves the requested bin count.
    pub fn histogram_ranges(
        &self,
        column: u32,
        filters: Option<&ViewState>,
        bins: u32,
    ) -> Result<crate::RangeHistogram> {
        let array = self.column(column)?;
        let rows = self.rows_in(RowScope::facet(filters, column))?;
        histogram::ranges(array, &rows, bins)
    }

    /// Validate and prepare an explicit filter without scanning data. Value-list
    /// filters may build the column's lazy index; other filters only parse scalars.
    pub fn validate_filter(&self, filter: &Filter) -> Result<()> {
        self.prepare_filter(filter).map(|_| ())
    }

    /// Keys for the given displayed labels (`None` = null). Unknown labels
    /// are ignored. Used by the CLI and MCP, which filter by value text.
    pub fn keys_for_labels(&self, column: u32, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let idx = self.keyed_index(column)?;
        Ok(idx
            .labels
            .iter()
            .enumerate()
            .filter(|(_, l)| labels.contains(l))
            .map(|(k, _)| k as u64)
            .collect())
    }

    pub(super) fn prepare_filter(&self, filter: &Filter) -> Result<Predicate<'_>> {
        let column = Column::new(self.column(filter.column())?);
        match filter {
            Filter::InValues { keys, .. } => {
                Ok(column.prepare_values(self.keyed_index(filter.column())?, keys))
            }
            _ => column.prepare_filter(filter),
        }
    }
}
