//! Immutable filtered and ordered views over shared source data.

use std::{collections::HashSet, sync::Arc};

use arrow::array::ArrayRef;

use crate::{
    col_view::ColView,
    column::Column,
    execution::sort::{codes_match_arrow_order, sort_rows, SortKey},
    row_ids::RowIds,
    ColumnId, ColumnSort, Dataset, Result,
};

mod analysis;
mod column_segment;
mod export;
mod filter;
mod find;
mod selection;

pub use column_segment::{FormattedColumnSegment, SourceRow, SourceRows, ViewRow};
pub(crate) use selection::FilterSelection;

/// Immutable snapshot of filters applied to a [`Dataset`].
/// Owns the selected rows and the information needed to derive facets.
/// Provides column analysis, filter metadata, and sorting.
pub struct FilteredView {
    dataset: Arc<Dataset>,
    rows: RowIds,
    /// Filter bitmaps behind `rows`; empty for an unfiltered view.
    selection: FilterSelection,
}

/// Ordered row projection over an immutable [`Dataset`].
/// Owns display row order independently of [`FilteredView`] and provides row
/// formatting, cell Find, and export.
pub struct OrderedView {
    dataset: Arc<Dataset>,
    rows: RowIds,
}

impl FilteredView {
    pub(crate) fn all(dataset: Arc<Dataset>) -> Arc<Self> {
        let n_rows = dataset.n_rows();
        Arc::new(Self {
            dataset,
            rows: RowIds::All(n_rows),
            selection: FilterSelection::default(),
        })
    }

    /// A view of the rows selected by `selection`'s bitmaps, materializing their row IDs once.
    pub(crate) fn selected(dataset: Arc<Dataset>, selection: FilterSelection) -> Arc<Self> {
        let Some(mask) = selection.mask() else {
            return Self::all(dataset);
        };
        let rows = RowIds::Selected(FilterSelection::row_ids(mask).into());
        Arc::new(Self {
            dataset,
            rows,
            selection,
        })
    }

    /// Derive a view of the source rows matching all filters in this snapshot
    /// except the one on `column`. The result may include rows absent from `self`.
    /// Returns the same view if `column` has no filter. Calling `facet` on the
    /// derived view removes another filter without restoring the first.
    pub fn facet(self: &Arc<Self>, column: ColumnId) -> Result<Arc<Self>> {
        self.dataset.column(column)?;
        if !self.selection.constrains(column) {
            return Ok(self.clone());
        }
        Ok(Self::selected(
            self.dataset.clone(),
            self.selection.without(column),
        ))
    }

    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    /// Produce an [`OrderedView`] preserving this view's row order in O(1).
    pub fn unsorted(&self) -> Arc<OrderedView> {
        Arc::new(OrderedView {
            dataset: self.dataset.clone(),
            rows: self.rows.clone(),
        })
    }

    /// Sort this view's rows independently, leaving `self` unchanged.
    /// Empty sort keys return [`Self::unsorted`] sharing the same row selection.
    pub fn sort(&self, sorts: &[ColumnSort]) -> Result<Arc<OrderedView>> {
        let mut seen = HashSet::with_capacity(sorts.len());
        for sort in sorts {
            self.dataset.column(sort.column)?;
            if !seen.insert(sort.column) {
                return Err(crate::ColumnError::Duplicate {
                    operation: crate::ColumnOperation::Sort,
                    column: sort.column,
                }
                .into());
            }
        }
        if sorts.is_empty() {
            return Ok(self.unsorted());
        }

        let mut keys = Vec::with_capacity(sorts.len());
        for sort in sorts {
            let column = self.column(sort.column)?;
            let codes = if codes_match_arrow_order(column.array().data_type()) {
                column.dictionary()?.map(|dictionary| dictionary.as_ref())
            } else {
                None
            };
            keys.push(SortKey {
                array: column.array(),
                descending: sort.descending,
                codes,
            });
        }
        let sorted = sort_rows(self.rows.as_slice(), &keys)?;
        Ok(Arc::new(OrderedView {
            dataset: self.dataset.clone(),
            rows: RowIds::Selected(sorted.into()),
        }))
    }

    pub(crate) fn column(&self, column: ColumnId) -> Result<&Column, crate::ColumnError> {
        self.dataset.column(column)
    }

    pub(crate) fn column_array(&self, column: ColumnId) -> Result<&ArrayRef, crate::ColumnError> {
        Ok(self.column(column)?.array())
    }

    pub(crate) fn col(&self, column: ColumnId) -> Result<ColView<'_>, crate::ColumnError> {
        let mask = if self.rows.len() == self.dataset.n_rows() {
            None
        } else {
            self.selection.mask()
        };
        Ok(ColView::from_mask(self.column(column)?, mask))
    }
}

impl OrderedView {
    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    pub(crate) fn column(&self, column: ColumnId) -> Result<&Column, crate::ColumnError> {
        self.dataset.column(column)
    }

    pub(crate) fn column_array(&self, column: ColumnId) -> Result<&ArrayRef, crate::ColumnError> {
        Ok(self.column(column)?.array())
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::{Int32Array, RecordBatch, StringArray};

    use super::*;
    use crate::{AxisScale, ColumnError, ColumnFilter, ColumnOperation, Error, Filter};

    fn dataset() -> Arc<Dataset> {
        let numbers: ArrayRef = Arc::new(Int32Array::from_iter_values(0..12_000));
        let labels: ArrayRef = Arc::new(StringArray::from_iter_values((0..12_000).map(|row| {
            if row % 2 == 0 {
                "even"
            } else {
                "odd"
            }
        })));
        let batch = RecordBatch::try_from_iter([("number", numbers), ("label", labels)]).unwrap();
        Dataset::from_test_batch(batch)
    }

    #[test]
    fn unfiltered_and_unsorted_keep_zero_alloc_all_rows_across_operations() {
        let ds = dataset();
        let c0 = ds.column_id("number").unwrap();
        let c1 = ds.column_id("label").unwrap();

        let unfiltered = ds.unfiltered();
        assert!(matches!(unfiltered.rows, RowIds::All(12_000)));
        assert_eq!(unfiltered.row_count(), 12_000);

        let empty_filter = ds.filter(&[]).unwrap();
        assert!(matches!(empty_filter.rows, RowIds::All(12_000)));
        assert!(unfiltered.rows.ptr_eq(&empty_filter.rows));

        let unsorted = unfiltered.unsorted();
        assert!(matches!(unsorted.rows, RowIds::All(12_000)));
        assert_eq!(unsorted.row_count(), 12_000);
        assert_eq!(
            unsorted.display_row_for_source(SourceRow(11999)),
            Some(ViewRow(11999))
        );
        assert_eq!(unsorted.display_row_for_source(SourceRow(12000)), None);

        let empty_sort = unfiltered.sort(&[]).unwrap();
        assert!(matches!(empty_sort.rows, RowIds::All(12_000)));

        // Works across rows, stats, find, and export.
        assert_eq!(unsorted.source_rows(0, 3), [0, 1, 2]);
        assert_eq!(unsorted.format_column(c0, 0, 3).unwrap().cell(0), Some("0"));
        assert_eq!(
            unsorted.format_column(c1, 0, 3).unwrap().cell(1),
            Some("odd")
        );

        assert_eq!(unfiltered.count_nulls(c0).unwrap(), 0);
        assert_eq!(unfiltered.statistics(c0).unwrap().mean, Some(5999.5));
        assert_eq!(
            unfiltered
                .distribution(c0, 4, AxisScale::Linear)
                .unwrap()
                .counts
                .iter()
                .sum::<u64>(),
            12_000
        );
        assert_eq!(unfiltered.values(c1).unwrap().distinct_count, 2);

        let find = unsorted.find("odd", &[c1]).unwrap();
        let hits = find.next(None, 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].display_row, 1);
        assert_eq!(hits[0].column, c1);

        #[cfg(feature = "csv")]
        {
            let dir = tempfile::tempdir().unwrap();
            let export_path = dir.path().join("all.csv");
            let export = unsorted.export(&[c0, c1], crate::FileType::Csv).unwrap();
            assert_eq!(export.progress().total_rows, 12_000);
            export.write(&export_path).unwrap();
            assert_eq!(export.progress().rows_written, 12_000);
        }
    }

    #[test]
    fn repeated_sorts_on_one_filtered_view_are_independent_and_empty_sort_shares_rows() {
        let ds = dataset();
        let c0 = ds.column_id("number").unwrap();
        let c1 = ds.column_id("label").unwrap();

        let filtered = ds
            .filter(&[ColumnFilter {
                column: c0,
                filter: Filter::range(Some("11997".into()), None, false),
            }])
            .unwrap();
        assert_eq!(filtered.row_count(), 3);
        assert!(matches!(filtered.rows, RowIds::Selected(_)));

        let unsort_1 = filtered.unsorted();
        let unsort_2 = filtered.sort(&[]).unwrap();
        assert!(unsort_1.rows.ptr_eq(&filtered.rows));
        assert!(unsort_2.rows.ptr_eq(&filtered.rows));
        assert_eq!(unsort_1.source_rows(0, 3), [11997, 11998, 11999]);

        let desc = filtered
            .sort(&[ColumnSort {
                column: c0,
                descending: true,
            }])
            .unwrap();
        let by_label_then_num = filtered
            .sort(&[
                ColumnSort {
                    column: c1,
                    descending: false,
                },
                ColumnSort {
                    column: c0,
                    descending: true,
                },
            ])
            .unwrap();

        assert_eq!(desc.source_rows(0, 3), [11999, 11998, 11997]);
        assert_eq!(
            desc.display_row_for_source(SourceRow(11999)),
            Some(ViewRow(0))
        );
        assert_eq!(
            desc.display_row_for_source(SourceRow(11997)),
            Some(ViewRow(2))
        );
        assert_eq!(desc.display_row_for_source(SourceRow(0)), None);
        assert_eq!(desc.display_row_for_source(SourceRow(u64::MAX)), None);
        assert_eq!(by_label_then_num.source_rows(0, 3), [11998, 11999, 11997]);
        // Original filtered and unsorted views remain unaffected.
        assert_eq!(unsort_1.source_rows(0, 3), [11997, 11998, 11999]);
    }

    #[test]
    fn facet_drops_only_its_own_condition_and_matches_refiltering() {
        let ds = dataset();
        let c0 = ds.column_id("number").unwrap();
        let c1 = ds.column_id("label").unwrap();
        let number = ColumnFilter {
            column: c0,
            filter: Filter::range(Some("100".into()), Some("9000".into()), false),
        };
        let label = ColumnFilter {
            column: c1,
            filter: Filter::exact(vec!["odd".into()], false, false),
        };
        let filtered = ds.filter(&[number.clone(), label.clone()]).unwrap();
        assert_eq!(filtered.row_count(), 4_450);

        let source_rows = |view: &FilteredView| view.rows.as_slice().iter_u32().collect::<Vec<_>>();
        let number_facet = filtered.facet(c0).unwrap();
        assert_eq!(
            source_rows(&number_facet),
            source_rows(&ds.filter(std::slice::from_ref(&label)).unwrap())
        );
        let label_facet = filtered.facet(c1).unwrap();
        assert_eq!(
            source_rows(&label_facet),
            source_rows(&ds.filter(std::slice::from_ref(&number)).unwrap())
        );
        let choices = label_facet.filter_choices(c1).unwrap().values.unwrap();
        assert_eq!(choices.iter().map(|c| c.count).collect::<Vec<_>>(), [
            4_451, 4_450
        ]);

        // An unfiltered column keeps the view; the last condition yields all rows.
        let only_number = ds.filter(std::slice::from_ref(&number)).unwrap();
        assert!(Arc::ptr_eq(&only_number.facet(c1).unwrap(), &only_number));
        assert!(matches!(
            only_number.facet(c0).unwrap().rows,
            RowIds::All(12_000)
        ));
        assert!(matches!(
            number_facet.facet(c1).unwrap().rows,
            RowIds::All(12_000)
        ));
        let unfiltered = ds.unfiltered();
        assert!(Arc::ptr_eq(&unfiltered.facet(c0).unwrap(), &unfiltered));
        assert!(unfiltered.facet(ColumnId::new(9)).is_err());
    }

    #[test]
    fn duplicate_columns_in_filter_and_sort_return_specific_operation() {
        let ds = dataset();
        let c0 = ds.column_id("number").unwrap();
        let dup_filter = ds.filter(&[
            ColumnFilter {
                column: c0,
                filter: Filter::range(Some("1".into()), None, false),
            },
            ColumnFilter {
                column: c0,
                filter: Filter::range(None, Some("10".into()), false),
            },
        ]);
        assert!(matches!(
            dup_filter,
            Err(Error::Column(ColumnError::Duplicate {
                operation: ColumnOperation::Filter,
                column,
            })) if column == c0
        ));

        let unfiltered = ds.unfiltered();
        let dup_sort = unfiltered.sort(&[
            ColumnSort {
                column: c0,
                descending: false,
            },
            ColumnSort {
                column: c0,
                descending: true,
            },
        ]);
        assert!(matches!(
            dup_sort,
            Err(Error::Column(ColumnError::Duplicate {
                operation: ColumnOperation::Sort,
                column,
            })) if column == c0
        ));
    }

    #[test]
    fn ordered_view_stays_valid_after_dropping_filtered_view_and_dataset() {
        let ds = dataset();
        let c0 = ds.column_id("number").unwrap();
        let c1 = ds.column_id("label").unwrap();
        let filtered = ds
            .filter(&[ColumnFilter {
                column: c0,
                filter: Filter::range(Some("11998".into()), None, false),
            }])
            .unwrap();
        let ordered = filtered
            .sort(&[ColumnSort {
                column: c0,
                descending: true,
            }])
            .unwrap();
        let find = ordered.find("odd", &[c1]).unwrap();
        drop(filtered);
        drop(ds);
        assert_eq!(ordered.row_count(), 2);
        assert_eq!(ordered.source_rows(0, 2), [11999, 11998]);
        assert_eq!(find.next(None, 10).unwrap()[0].display_row, 0);
    }

    #[test]
    fn column_info_carries_accurate_id_and_analysis_capabilities() {
        let ds = dataset();
        let info = ds.info();
        assert_eq!(info.columns.len(), 2);
        assert_eq!(info.columns[0].id, ColumnId::new(0));
        assert_eq!(info.columns[0].name, "number");
        assert!(info.columns[0].analysis.statistics);
        assert!(info.columns[0].analysis.distribution);
        assert!(info.columns[0].analysis.values);
        assert!(!info.columns[0].analysis.lengths);
        assert!(!info.columns[0].analysis.structure);

        assert_eq!(info.columns[1].id, ColumnId::new(1));
        assert_eq!(info.columns[1].name, "label");
        assert!(!info.columns[1].analysis.statistics);
        assert!(!info.columns[1].analysis.distribution);
        assert!(info.columns[1].analysis.values);
        assert!(info.columns[1].analysis.lengths);
        assert!(!info.columns[1].analysis.structure);
    }
}
