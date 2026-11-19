//! Capture validated export snapshots from an [`OrderedView`].

use std::{collections::BTreeSet, ops::Range, sync::Arc};

use super::OrderedView;
use crate::{row_ids::RowIds, ColumnId, Export, FileType, Result};

impl OrderedView {
    /// Capture a view and column order for export without copying its cells or
    /// row indices. The returned job remains valid when the current view changes.
    pub fn export(&self, columns: &[ColumnId], format: FileType) -> Result<Export> {
        self.capture_export(self.rows.clone(), columns, format)
    }

    /// Like [`Self::export`], limited to display-row ranges in order, such as
    /// a table selection. Ranges are clipped to the available rows. Only the
    /// selected source row indices are copied; cells are not.
    pub fn export_rows(
        &self,
        ranges: &[Range<u64>],
        columns: &[ColumnId],
        format: FileType,
    ) -> Result<Export> {
        let len = self.rows.len() as u64;
        let slice = self.rows.as_slice();
        let mut rows = Vec::new();
        for range in ranges {
            let start = range.start.min(len);
            let end = range.end.min(len).max(start);
            rows.extend(
                slice
                    .window(start as usize, end as usize)
                    .iter()
                    .map(|row| row as u32),
            );
        }
        self.capture_export(RowIds::Selected(rows.into()), columns, format)
    }

    fn capture_export(
        &self,
        rows: RowIds,
        columns: &[ColumnId],
        format: FileType,
    ) -> Result<Export> {
        format.require_enabled()?;
        if columns.is_empty() {
            return Err(crate::ColumnError::EmptySelection {
                operation: crate::ColumnOperation::Export,
            }
            .into());
        }
        let mut seen = BTreeSet::new();
        let arrays = columns
            .iter()
            .map(|&column| {
                if !seen.insert(column) {
                    return Err(crate::ColumnError::Duplicate {
                        operation: crate::ColumnOperation::Export,
                        column,
                    });
                }
                self.column_array(column).cloned()
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let indices = columns.iter().map(|c| c.index()).collect::<Vec<_>>();
        let schema = Arc::new(self.dataset.schema.project(&indices)?);
        if format != FileType::Parquet {
            let nested: Vec<ColumnId> = columns
                .iter()
                .copied()
                .filter(|&column| {
                    self.dataset.columns()[column.index()]
                        .array()
                        .data_type()
                        .is_nested()
                })
                .collect();
            if !nested.is_empty() {
                return Err(crate::ExportError::NestedColumns {
                    format,
                    columns: nested,
                }
                .into());
            }
        }
        Ok(Export::new(
            self.dataset.source_path.clone(),
            schema,
            arrays,
            rows,
            format,
        ))
    }

    /// Names of selected nested columns, which require Parquet for export.
    /// Inspect types only; this does not scan or copy any cells.
    pub fn nested_export_columns(&self, columns: &[ColumnId]) -> Result<Vec<String>> {
        let mut nested = Vec::new();
        for &column in columns {
            if self.column(column)?.array().data_type().is_nested() {
                nested.push(self.dataset.schema.field(column.index()).name().clone());
            }
        }
        Ok(nested)
    }
}
