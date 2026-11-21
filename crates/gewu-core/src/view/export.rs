//! Capture validated export snapshots from an [`OrderedView`].

use std::{collections::BTreeSet, sync::Arc};

use super::OrderedView;
use crate::{row_ids::RowIds, ColumnId, Export, FileType, Result};

impl OrderedView {
    /// Capture every row of this view and a column order for export without
    /// copying cells or row indices. The job stays valid when the current view
    /// changes. Nested columns are checked when writing, where
    /// [`crate::ExportOptions::nested_as_text`] decides whether they are allowed.
    pub fn export(&self, columns: &[ColumnId], format: FileType) -> Result<Export> {
        self.export_with_rows(self.rows.clone(), columns, format)
    }

    /// Capture the display rows in `ranges`, in order, for export. Ranges are
    /// clipped to the available rows; overlapping ranges repeat their rows.
    pub fn export_rows(
        &self,
        ranges: &[std::ops::Range<u64>],
        columns: &[ColumnId],
        format: FileType,
    ) -> Result<Export> {
        let mut rows = Vec::new();
        for range in ranges {
            let count = u32::try_from(range.end.saturating_sub(range.start)).unwrap_or(u32::MAX);
            rows.extend(self.row_window(range.start, count).iter_u32());
        }
        self.export_with_rows(RowIds::Selected(rows.into()), columns, format)
    }

    fn export_with_rows(
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
        Ok(Export::new(
            self.dataset.source_path.clone(),
            schema,
            arrays,
            columns.to_vec(),
            rows,
            format,
        ))
    }
}
