//! The export dialog captures the displayed view and visible column order when it opens.
//! Confirming hands the draft to the document, which owns the export job.
use std::{path::PathBuf, sync::Arc};

use gewu_core::{ColumnId, FileType, OrderedView, ParquetCompression};
use gpui_kit::*;

use super::{Drawer, Preferences, TableView, button};
use crate::{
    panels::choice::{Choice, ChoiceField},
    table::TableEvent,
};

#[derive(Clone)]
pub(crate) struct ExportDraft {
    pub(crate) view: Arc<OrderedView>,
    pub(crate) source: PathBuf,
    pub(crate) columns: Vec<ColumnId>,
    pub(crate) format: FileType,
    pub(crate) compression: ParquetCompression,
}

const FORMATS: [FileType; 4] = [
    FileType::Parquet,
    FileType::Csv,
    FileType::Tsv,
    FileType::Xlsx,
];
const COMPRESSIONS: [(ParquetCompression, &str); 6] = [
    (ParquetCompression::Zstd, "Zstd"),
    (ParquetCompression::Snappy, "Snappy"),
    (ParquetCompression::Gzip, "Gzip"),
    (ParquetCompression::Brotli, "Brotli"),
    (ParquetCompression::Lz4, "LZ4"),
    (ParquetCompression::Uncompressed, "Uncompressed"),
];

impl TableView {
    /// A settled query with at least one visible column can be exported.
    pub(crate) fn can_export(&self) -> bool {
        !self.data.queries().is_running() && !self.data.columns.is_empty()
    }

    pub(crate) fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_export() {
            return;
        }
        self.focus_panel(window, cx);
        let prefs = Preferences::get(cx);
        self.controls.menu = None;
        self.controls.drawer = Some(Drawer::Export(ExportDraft {
            view: self.data.view().clone(),
            source: self.data.snapshot().dataset().path().to_owned(),
            columns: self.data.column_ids(0..self.data.columns.len()),
            format: if prefs.remember_export_format {
                prefs.last_export_format
            } else {
                FileType::Parquet
            },
            compression: prefs.parquet_compression,
        }));
        cx.notify();
    }

    pub(super) fn export_panel(
        &self,
        draft: &ExportDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let form_row = |label: &'static str, field: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(92.)).flex_none().child(label))
                .child(div().flex_1().min_w_0().child(field))
        };
        let format = ChoiceField {
            id: "export-format".into(),
            label: "File format",
            searchable: false,
            options: FORMATS
                .iter()
                .map(|format| Choice {
                    value: *format,
                    label: crate::ui::messages::format_name(*format).into(),
                })
                .collect(),
            selected: Some(draft.format),
        };
        let pick = cx.listener(|t, chosen: &FileType, _, cx| {
            if let Some(Drawer::Export(d)) = &mut t.controls.drawer {
                d.format = *chosen;
                cx.notify();
            }
        });
        let mut panel = self
            .drawer("Export", window, cx)
            .child(
                div()
                    .text_color(crate::theme::GewuPalette::get(cx).muted_foreground)
                    .child(format!(
                        "{} · {}",
                        crate::counted(draft.view.row_count(), "row"),
                        crate::counted(draft.columns.len() as u64, "visible column")
                    )),
            )
            .child(form_row("Format", format.element(window, cx, pick)));
        if draft.format == FileType::Parquet {
            let compression = ChoiceField {
                id: "export-compression".into(),
                label: "Parquet compression",
                searchable: false,
                options: COMPRESSIONS
                    .iter()
                    .map(|(value, label)| Choice {
                        value: *value,
                        label: (*label).into(),
                    })
                    .collect(),
                selected: Some(draft.compression),
            };
            let pick = cx.listener(|t, chosen: &ParquetCompression, _, cx| {
                if let Some(Drawer::Export(d)) = &mut t.controls.drawer {
                    d.compression = *chosen;
                    cx.notify();
                }
            });
            panel = panel.child(form_row(
                "Compression",
                compression.element(window, cx, pick),
            ));
        }
        panel
            .child(
                super::footer()
                    .child(self.close_button("Cancel", cx))
                    .child(button("choose-export-path", "Export…", true, cx).on_click(
                        cx.listener(|t, _, w, cx| {
                            w.focus(&t.focus, cx);
                            t.submit_export(cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    pub(super) fn submit_export(&mut self, cx: &mut Context<Self>) {
        let Some(Drawer::Export(draft)) = self.controls.drawer.take() else {
            return;
        };
        cx.emit(TableEvent::Export(draft));
        cx.notify();
    }
}
