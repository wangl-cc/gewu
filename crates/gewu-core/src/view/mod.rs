//! Immutable row and column projections over shared source data.

use std::sync::Arc;

use arrow::array::ArrayRef;

use crate::{column::Column, scope::Selection, Dataset, Error, Result};

mod analysis;
pub mod choices;
mod export;
mod filter;
mod find;
mod query;

pub use choices::{TextChoice, TextChoiceCursor, TextChoicePage, TextChoices};

/// A query result. Query conditions and current-window state belong to callers.
pub struct View {
    dataset: Arc<Dataset>,
    columns: Box<[u32]>,
    rows: Arc<[u32]>,
}

impl View {
    pub(crate) fn all(dataset: Arc<Dataset>) -> Arc<Self> {
        Arc::new(Self {
            columns: (0..dataset.columns().len() as u32).collect(),
            rows: (0..dataset.n_rows() as u32).collect(),
            dataset,
        })
    }

    pub fn dataset(&self) -> &Arc<Dataset> {
        &self.dataset
    }

    pub fn columns(&self) -> &[u32] {
        &self.columns
    }

    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    /// Change only display column order. The order must be a complete permutation.
    pub fn reorder_columns(&self, order: &[u32]) -> Result<Arc<Self>> {
        let mut expected = self.columns.to_vec();
        let mut actual = order.to_vec();
        expected.sort_unstable();
        actual.sort_unstable();
        if actual != expected {
            return Err(Error::InvalidArgument(
                "column order must be a permutation of the view's columns".into(),
            ));
        }
        Ok(Arc::new(Self {
            dataset: self.dataset.clone(),
            columns: order.into(),
            rows: self.rows.clone(),
        }))
    }

    pub(crate) fn column(&self, column: u32) -> Result<&Column> {
        self.dataset.column(column)
    }

    pub(crate) fn column_array(&self, column: u32) -> Result<&ArrayRef> {
        Ok(self.column(column)?.array())
    }

    // Order-independent scans can use contiguous buffers for an unfiltered view.
    fn selection(&self) -> Selection {
        if self.rows.len() == self.dataset.n_rows() {
            Selection::All(self.rows.len())
        } else {
            Selection::Rows(self.rows.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::{Int32Array, RecordBatch, StringArray};

    use super::*;
    use crate::{Cancellation, ColSpec, Filter, QuerySpec};

    fn dataset() -> Arc<Dataset> {
        let numbers: ArrayRef = Arc::new(Int32Array::from_iter_values(0..12_000));
        let labels: ArrayRef = Arc::new(StringArray::from_iter_values((0..12_000).map(|row| {
            if row % 2 == 0 {
                "even"
            } else {
                "odd"
            }
        })));
        let batch =
            RecordBatch::try_from_iter([("number", numbers.clone()), ("label", labels.clone())])
                .unwrap();
        Arc::new(Dataset {
            source_path: Default::default(),
            schema: batch.schema(),
            columns: vec![
                Column::new("number", numbers).unwrap(),
                Column::new("label", labels).unwrap(),
            ],
            n_rows: 12_000,
            file_size: 0,
            format: crate::FileType::Csv,
            metadata: crate::FileMetadata::Delimited,
        })
    }

    #[test]
    fn reorder_shares_rows_and_dataset_and_requires_an_exact_permutation() {
        let dataset = dataset();
        let initial = dataset.view();
        let sorted = initial
            .query(
                &QuerySpec::all(2).with_sort(0, true, 0),
                &Cancellation::default(),
            )
            .unwrap();
        let reordered = sorted.reorder_columns(&[1, 0]).unwrap();
        assert!(Arc::ptr_eq(&sorted.dataset, &reordered.dataset));
        assert!(Arc::ptr_eq(&sorted.rows, &reordered.rows));
        assert_eq!(sorted.columns(), &[0, 1]);
        assert_eq!(reordered.columns(), &[1, 0]);
        assert_eq!(
            reordered.rows(0, 2, reordered.columns()).unwrap().cells,
            ["odd", "11999", "even", "11998"].map(|text| Some(text.to_owned()))
        );
        for order in [&[][..], &[0], &[0, 0], &[0, 1, 2], &[1, 2]] {
            assert!(matches!(
                sorted.reorder_columns(order),
                Err(Error::InvalidArgument(_))
            ));
        }
        assert_eq!(initial.rows(0, 2, &[0]).unwrap().source_rows, [0, 1]);
    }

    #[test]
    fn independent_queries_keep_snapshots_and_recover_hidden_rows_and_columns() {
        let dataset = dataset();
        let initial = dataset.view();
        let spec = QuerySpec::all(2)
            .with_filter(0, Filter::range(Some("11998".into()), None, false))
            .with_sort(0, true, 0);
        let filtered = initial.query(&spec, &Cancellation::default()).unwrap();
        let choices = filtered.text_choices(1).unwrap();
        let find = filtered.find("odd", &[1]).unwrap();
        let hidden = filtered
            .query(
                &QuerySpec {
                    columns: vec![ColSpec {
                        column: 1,
                        sort: None,
                        filter: None,
                    }],
                },
                &Cancellation::default(),
            )
            .unwrap();
        assert_eq!(hidden.columns(), &[1]);
        assert_eq!(hidden.row_count(), 12_000);
        assert!(Arc::ptr_eq(filtered.dataset(), hidden.dataset()));
        let restored = hidden
            .query(&QuerySpec::all(2), &Cancellation::default())
            .unwrap();
        assert_eq!(restored.rows(0, 2, &[0]).unwrap().source_rows, [0, 1]);
        assert_eq!(filtered.rows(0, 2, &[0]).unwrap().source_rows, [
            11999, 11998
        ]);
        drop(dataset);
        drop(initial);
        drop(filtered);
        assert_eq!(
            find.next(None, 10, &Cancellation::default()).unwrap()[0].display_row,
            0
        );
        let page = choices
            .page("", None, 10, &Cancellation::default())
            .unwrap();
        assert_eq!(
            page.values
                .iter()
                .map(|value| (value.value.as_str(), value.count))
                .collect::<Vec<_>>(),
            [("even", 1), ("odd", 1)]
        );
        assert_eq!(
            restored.count_nulls(0, &Cancellation::default()).unwrap(),
            0
        );
    }

    #[test]
    fn cancelled_and_invalid_queries_leave_existing_views_usable() {
        let initial = dataset().view();
        let spec = QuerySpec::all(2).with_sort(0, true, 0);
        let token = Cancellation::default();
        token.cancel();
        assert!(matches!(
            initial.query(&spec, &token),
            Err(Error::Cancelled)
        ));
        // Enter the real row scan, then stop at its next checkpoint.
        let interrupted = Cancellation::after_checks(4);
        assert!(matches!(
            initial.query(&spec, &interrupted),
            Err(Error::Cancelled)
        ));
        let sorted = initial.query(&spec, &Cancellation::default()).unwrap();
        assert_eq!(sorted.rows(0, 1, &[0]).unwrap().source_rows, [11999]);
        for bad in [
            QuerySpec::single(5, Filter::exact(vec![], false, false)),
            QuerySpec {
                columns: vec![
                    ColSpec {
                        column: 0,
                        sort: None,
                        filter: None
                    };
                    2
                ],
            },
            QuerySpec::all(2)
                .with_sort(0, false, 0)
                .with_sort(1, false, 0),
            QuerySpec::single(0, Filter::exact(vec!["invalid".into()], false, false)),
        ] {
            assert!(matches!(
                initial.query(&bad, &Cancellation::default()),
                Err(Error::InvalidArgument(_))
            ));
        }
        assert_eq!(initial.rows(0, 1, &[0]).unwrap().source_rows, [0]);
        assert_eq!(sorted.row_count(), 12_000);
    }
}
