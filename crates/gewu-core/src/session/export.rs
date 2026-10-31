//! Capture validated export snapshots from a document.

use std::{collections::BTreeSet, sync::Arc};

use super::Session;
use crate::{Error, Export, FileType, Result};

impl Session {
    /// Capture a view and column order for export without copying its cells or
    /// row indices. The returned job remains valid when the current view changes.
    pub fn prepare_export(
        &self,
        version: u64,
        columns: &[u32],
        format: FileType,
    ) -> Result<Export> {
        format.require_enabled()?;
        let view = self.views.snapshot(version)?;
        if columns.is_empty() {
            return Err(Error::InvalidArgument(
                "select at least one column to export".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let arrays = columns
            .iter()
            .map(|&column| {
                if !seen.insert(column) {
                    return Err(Error::InvalidArgument("duplicate export column".into()));
                }
                self.column(column).cloned()
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = columns.iter().map(|&c| c as usize).collect::<Vec<_>>();
        let schema = Arc::new(self.dataset.schema.project(&indices)?);
        if format != FileType::Parquet {
            let nested = self.nested_export_columns(columns)?;
            if !nested.is_empty() {
                return Err(Error::Unsupported(format!(
                    "{format} cannot export nested columns ({}); use Parquet to preserve them",
                    nested.join(", ")
                )));
            }
        }
        Ok(Export::new(
            self.dataset.source_path.clone(),
            schema,
            arrays,
            view.rows.clone(),
            format,
        ))
    }

    /// Names of selected nested columns, which require Parquet for export.
    /// Inspect types only; this does not scan or copy any cells.
    pub fn nested_export_columns(&self, columns: &[u32]) -> Result<Vec<String>> {
        let mut nested = Vec::new();
        for &column in columns {
            if self.column(column)?.data_type().is_nested() {
                nested.push(self.dataset.schema.field(column as usize).name().clone());
            }
        }
        Ok(nested)
    }
}
