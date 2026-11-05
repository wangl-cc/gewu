//! Capture validated export snapshots from a document.

use std::{collections::BTreeSet, sync::Arc};

use super::View;
use crate::{Export, FileType, Result};

impl View {
    /// Capture a view and column order for export without copying its cells or
    /// row indices. The returned job remains valid when the current view changes.
    pub fn export(&self, columns: &[u32], format: FileType) -> Result<Export> {
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
        let indices = columns.iter().map(|&c| c as usize).collect::<Vec<_>>();
        let schema = Arc::new(self.dataset.schema.project(&indices)?);
        if format != FileType::Parquet {
            let nested: Vec<u32> = columns
                .iter()
                .copied()
                .filter(|&column| {
                    self.dataset.columns[column as usize]
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
            self.rows.clone(),
            format,
        ))
    }

    /// Names of selected nested columns, which require Parquet for export.
    /// Inspect types only; this does not scan or copy any cells.
    pub fn nested_export_columns(&self, columns: &[u32]) -> Result<Vec<String>> {
        let mut nested = Vec::new();
        for &column in columns {
            if self.column(column)?.array().data_type().is_nested() {
                nested.push(self.dataset.schema.field(column as usize).name().clone());
            }
        }
        Ok(nested)
    }
}
