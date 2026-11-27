//! A document's export job: choosing the destination, writing the captured view and
//! reporting progress. The job outlives the table it was captured from, so replacing the
//! displayed worksheet or import options does not interrupt it.
use std::{path::PathBuf, sync::Arc, time::Duration};

use gewu_core::{Export, ExportOptions, FileType};
use gpui_kit::*;

use crate::{preferences::Preferences, table::ExportDraft};

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

impl ExportController {
    /// A document runs one export at a time, from choosing its destination until it finishes.
    pub(super) fn is_busy(&self) -> bool {
        matches!(self, Self::Choosing { .. } | Self::Writing(_))
    }

    pub(super) fn choose_path(&mut self, draft: ExportDraft, cx: &mut Context<Self>) {
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
                *self = ExportController::Failed(crate::ui::errors::message(&error));
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
                    Err(error) => ExportController::Failed(crate::ui::errors::message(&error)),
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
                .text_color(crate::theme::GewuPalette::get(cx).danger_text)
                .child(crate::ui::errors::warning_icon())
                .child(status(message.clone().into())),
            ExportController::Writing(active) => {
                let progress = active.job.progress();
                bar.child(status(
                    format!(
                        "Exporting {} / {}",
                        crate::ui::messages::format_count(progress.rows_written),
                        crate::counted(progress.total_rows, "row")
                    )
                    .into(),
                ))
                .child(
                    crate::ui::status_button("cancel-export", "Cancel export", false, cx).on_click(
                        cx.listener(|t, _, _, _| {
                            if let ExportController::Writing(active) = t {
                                active.job.cancel();
                            }
                        }),
                    ),
                )
            }
        }
    }
}
