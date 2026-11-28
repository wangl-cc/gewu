//! A document's export job: choosing the destination and format, writing the captured view
//! and reporting progress. The job outlives the table it was captured from, so replacing the
//! displayed worksheet or import options does not interrupt it.
//!
//! On macOS the system save panel offers the formats in its format menu and Parquet's
//! compression below it. Elsewhere, and in tests, the toolkit's save prompt is used: the
//! chosen file name's extension selects the format, and Parquet uses the remembered
//! compression.
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use gewu_core::{Export, ExportOptions, FileType, ParquetCompression};
use gpui_kit::*;

use crate::{preferences::Preferences, table::ExportSource};

#[derive(Default)]
pub(super) enum ExportController {
    #[default]
    Idle,
    Choosing {
        _task: Task<()>,
    },
    Writing(WorkingExport),
    /// The name of the file written.
    Exported(String),
    Cancelled,
    Failed(ExportFailure),
}

pub(crate) enum ExportFailure {
    /// The save panel could not be shown.
    Destination(Box<dyn std::error::Error + Send + Sync>),
    Write(gewu_core::Error),
}

/// Where and how to write the captured view.
struct Destination {
    path: PathBuf,
    format: FileType,
    compression: ParquetCompression,
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

    /// Ask where and as what to export `source`, starting from the remembered format.
    pub(super) fn choose(&mut self, source: ExportSource, cx: &mut Context<Self>) {
        let prefs = Preferences::get(cx);
        let format = if prefs.remember_export_format {
            prefs.last_export_format
        } else {
            FileType::Parquet
        };
        let compression = prefs.parquet_compression;
        let directory = source.path.parent().unwrap_or(Path::new(".")).to_owned();
        let stem = format!(
            "{}-export",
            source
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let chosen = Self::destination(&source, &directory, &stem, format, compression, cx);
        *self = ExportController::Choosing {
            _task: cx.spawn(async move |this, cx| {
                let destination = chosen.await;
                let _ = this.update(cx, |model, cx| {
                    *model = ExportController::Idle;
                    match destination {
                        Ok(Some(destination)) => model.write_export(source, destination, cx),
                        Ok(None) => {}
                        Err(error) => *model = ExportController::Failed(error),
                    }
                    cx.notify();
                });
            }),
        };
        cx.notify();
    }

    /// The destination the person chooses, or `None` when they cancel.
    fn destination(
        source: &ExportSource,
        directory: &Path,
        stem: &str,
        format: FileType,
        compression: ParquetCompression,
        cx: &mut Context<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Destination>, ExportFailure>>>> {
        #[cfg(target_os = "macos")]
        if !cx.is_test() {
            return Box::pin(Self::save_panel(
                source,
                directory,
                stem,
                format,
                compression,
            ));
        }
        let _ = source;
        let receiver =
            cx.prompt_for_new_path(directory, Some(&format!("{stem}.{}", format.extension())));
        Box::pin(async move {
            match receiver.await {
                Ok(Ok(Some(path))) => Ok(Some(Destination {
                    // A name without a known extension keeps the suggested format.
                    format: FileType::from_path(&path).unwrap_or(format),
                    path,
                    compression,
                })),
                Ok(Ok(None)) | Err(_) => Ok(None),
                Ok(Err(error)) => Err(ExportFailure::Destination(error.into())),
            }
        })
    }

    #[cfg(target_os = "macos")]
    fn save_panel(
        source: &ExportSource,
        directory: &Path,
        stem: &str,
        format: FileType,
        compression: ParquetCompression,
    ) -> impl Future<Output = Result<Option<Destination>, ExportFailure>> + 'static {
        use crate::platform::macos::save::{ExportPanelOptions, choose_export};

        /// The formats offered, in menu order.
        const FORMATS: [FileType; 4] = [
            FileType::Parquet,
            FileType::Csv,
            FileType::Tsv,
            FileType::Xlsx,
        ];

        /// Parquet compressions offered, in menu order, with their names.
        const COMPRESSIONS: [(ParquetCompression, &str); 6] = [
            (ParquetCompression::Zstd, "Zstd"),
            (ParquetCompression::Snappy, "Snappy"),
            (ParquetCompression::Gzip, "Gzip"),
            (ParquetCompression::Brotli, "Brotli"),
            (ParquetCompression::Lz4, "LZ4"),
            (ParquetCompression::Uncompressed, "Uncompressed"),
        ];

        let formats: Vec<_> = FORMATS
            .iter()
            .map(|&format| (format, crate::ui::messages::format_choice_label(format)))
            .collect();
        let message = format!(
            "Export {} · visible columns in display order",
            crate::counted(source.view.row_count(), "row")
        );
        let options = ExportPanelOptions {
            title: "Export",
            message: &message,
            directory,
            stem,
            formats: &formats,
            format,
            compression_label: "Compression:",
            compressions: &COMPRESSIONS,
            compression,
        };
        let (sender, receiver) = async_channel::bounded(1);
        let shown = objc2::MainThreadMarker::new().is_some_and(|main| {
            choose_export(main, &options, move |choice| {
                let _ = sender.try_send(choice);
            })
        });
        async move {
            if !shown {
                return Err(ExportFailure::Destination(
                    "macOS could not open the save panel".into(),
                ));
            }
            Ok(receiver
                .recv()
                .await
                .ok()
                .flatten()
                .map(|choice| Destination {
                    path: choice.path,
                    format: choice.format,
                    compression: choice.compression,
                }))
        }
    }

    fn write_export(
        &mut self,
        source: ExportSource,
        destination: Destination,
        cx: &mut Context<Self>,
    ) {
        let Destination {
            path,
            format,
            compression,
        } = destination;
        let job = match source.view.export(&source.columns, format) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                *self = ExportController::Failed(ExportFailure::Write(error));
                return;
            }
        };
        let writer = job.clone();
        let options = ExportOptions {
            nested_as_text: true,
            parquet_compression: compression,
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
                            p.last_export_format = format;
                            if format == FileType::Parquet {
                                p.parquet_compression = compression;
                            }
                        });
                        ExportController::Exported(title)
                    }
                    Err(gewu_core::Error::Cancelled) => ExportController::Cancelled,
                    Err(error) => ExportController::Failed(ExportFailure::Write(error)),
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
            ExportController::Exported(name) => {
                bar.child(status(format!("Exported {name}").into()))
            }
            ExportController::Cancelled => bar.child(status("Export cancelled".into())),
            ExportController::Failed(failure) => bar
                .text_color(crate::theme::GewuPalette::get(cx).danger_text)
                .child(crate::ui::errors::warning_icon())
                .child(status(
                    crate::ui::errors::export_failure_message(failure).into(),
                )),
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
