use std::{path::PathBuf, sync::Arc, time::Duration};

use gewu_core::{ColumnId, Export, ExportOptions, FileType, OrderedView, ParquetCompression};
use gpui_kit::*;

use super::{Drawer, Preferences, TableView, button};
use crate::panels::choice::{Choice, ChoiceField};

pub(super) struct ExportDraft {
    view: Arc<OrderedView>,
    source: PathBuf,
    columns: Vec<ColumnId>,
    format: FileType,
    compression: ParquetCompression,
}

#[derive(Default)]
pub(super) enum ExportController {
    #[default]
    Idle,
    Choosing {
        _task: Task<()>,
    },
    Writing(WorkingExport),
    Finished(String),
    Failed(String),
}

pub(super) struct WorkingExport {
    job: Arc<Export>,
    _write: Task<()>,
    _progress: Task<()>,
}

// A closed document must stop its writer even when the blocking encoder has already started.
impl Drop for WorkingExport {
    fn drop(&mut self) {
        self.job.cancel();
    }
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
    pub(in crate::table) fn can_export(&self, cx: &App) -> bool {
        !self.data.queries().is_running()
            && !self.data.columns.is_empty()
            && !self.controls.export.as_ref().is_some_and(|e| {
                matches!(
                    e.read(cx),
                    ExportController::Choosing { .. } | ExportController::Writing(_)
                )
            })
    }

    pub(in crate::table) fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_export(cx) {
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
                    label: format.to_string().into(),
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
                    .text_color(
                        gpui_kit::base::Theme::global(cx)
                            .tokens
                            .colors
                            .muted_foreground,
                    )
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
                            t.choose_export_path(cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    pub(super) fn choose_export_path(&mut self, cx: &mut Context<Self>) {
        let Some(Drawer::Export(draft)) = self.controls.drawer.take() else {
            return;
        };
        let controller = self
            .controls
            .export
            .get_or_insert_with(|| cx.new(|_| ExportController::Idle))
            .clone();
        controller.update(cx, |model, cx| model.choose_path(draft, cx));
        cx.notify();
    }

    pub(super) fn export_status(&self, bar: Div, _: &mut Context<Self>) -> Div {
        // Mounting the same entity in the replacement table keeps its status and progress
        // subscribed.
        bar.children(self.controls.export.iter().cloned())
    }
}

impl ExportController {
    fn choose_path(&mut self, draft: ExportDraft, cx: &mut Context<Self>) {
        let stem = draft
            .source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let extension = draft.format.to_string().to_lowercase();
        let name = format!("{stem}-export.{extension}");
        let receiver = cx.prompt_for_new_path(
            draft.source.parent().unwrap_or(std::path::Path::new(".")),
            Some(&name),
        );
        *self = ExportController::Choosing {
            _task: cx.spawn(async move |this, cx| {
                let destination = receiver.await;
                let _ = this.update(cx, |model, cx| {
                    *model = ExportController::Idle;
                    match destination {
                        Ok(Ok(Some(path))) => model.write_export(draft, path, cx),
                        Ok(Err(e)) => {
                            *model = ExportController::Failed(format!(
                                "Could not choose export destination: {e}"
                            ))
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            }),
        };
        cx.notify();
    }

    fn write_export(&mut self, draft: ExportDraft, path: PathBuf, cx: &mut Context<Self>) {
        let job = match draft.view.export(&draft.columns, draft.format) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                *self = ExportController::Failed(crate::errors::message(&error));
                return;
            }
        };
        let writer = job.clone();
        let options = ExportOptions {
            nested_as_text: true,
            parquet_compression: draft.compression,
            ..Default::default()
        };
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let write = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { writer.write_with_options(path, &options) })
                .await;
            let _ = this.update(cx, |model, cx| {
                *model = match result {
                    Ok(()) => {
                        Preferences::update(cx, |p| {
                            p.last_export_format = draft.format;
                            if draft.format == FileType::Parquet {
                                p.parquet_compression = draft.compression;
                            }
                        });
                        ExportController::Finished(format!("Exported {title}"))
                    }
                    Err(gewu_core::Error::Cancelled) => {
                        ExportController::Finished("Export cancelled".into())
                    }
                    Err(error) => ExportController::Failed(crate::errors::message(&error)),
                };
                cx.notify();
            });
        });
        let progress = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        *self = ExportController::Writing(WorkingExport {
            job,
            _write: write,
            _progress: progress,
        });
        cx.notify();
    }
}

impl Render for ExportController {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = div().min_w_0().flex().items_center().gap_1();
        let status = |text: SharedString| div().min_w_0().truncate().child(text);
        match self {
            ExportController::Idle => bar,
            ExportController::Choosing { .. } => {
                bar.child(status("Choosing export destination…".into()))
            }
            ExportController::Finished(message) => bar.child(status(message.clone().into())),
            ExportController::Failed(message) => bar
                .text_color(base::Theme::global(cx).tokens.colors.destructive)
                .child(status(message.clone().into())),
            ExportController::Writing(active) => {
                let progress = active.job.progress();
                bar.child(status(
                    format!(
                        "Exporting {} / {}",
                        progress.rows_written,
                        crate::counted(progress.total_rows, "row")
                    )
                    .into(),
                ))
                .child(
                    button("cancel-export", "Cancel export", false, cx)
                        .flex_none()
                        .on_click(cx.listener(|t, _, _, _| {
                            if let ExportController::Writing(active) = t {
                                active.job.cancel();
                            }
                        })),
                )
            }
        }
    }
}
