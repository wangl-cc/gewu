//! What an export captures when it starts: the displayed view, the visible column order and
//! the source file. The document's export job chooses the destination and format.
use std::{path::PathBuf, sync::Arc};

use gewu_core::{ColumnId, OrderedView};

use super::TableView;

pub(crate) struct ExportSource {
    pub(crate) view: Arc<OrderedView>,
    pub(crate) columns: Vec<ColumnId>,
    /// The file the table was read from, which names and places the export.
    pub(crate) path: PathBuf,
}

impl TableView {
    /// A settled query with at least one visible column can be exported.
    pub(crate) fn can_export(&self) -> bool {
        !self.data.queries().is_running() && !self.data.columns.is_empty()
    }

    /// Capture the displayed rows and visible columns for an export starting now.
    pub(crate) fn export_source(&self) -> Option<ExportSource> {
        self.can_export().then(|| ExportSource {
            view: self.data.view().clone(),
            columns: self.data.columns.ids(),
            path: self.data.snapshot().dataset().path().to_owned(),
        })
    }
}
