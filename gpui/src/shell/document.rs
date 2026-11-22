//! Per-window import requests, replacement snapshots and document controls.
use std::{path::PathBuf, sync::Arc};

use gewu_core::{Dataset, FileMetadata, FileType, OpenOptions};
use gpui_kit::{base::Theme, *};

use super::{GetInfo, ImportOptions, info, open_paths, record_recent};
use crate::{data::TableData, panels::button, platform, table::TableView};

#[derive(Clone, PartialEq, Eq)]
struct Import {
    sheet: Option<String>,
    has_header: bool,
}

enum Load {
    Ready,
    Pending(Import),
    Failed(Import, gewu_core::Error),
}

pub(super) struct Document {
    path: PathBuf,
    title: SharedString,
    table: Option<Entity<TableView>>,
    sheets: Vec<String>,
    displayed: Import,
    header_available: bool,
    load: Load,
    reader: Option<Task<()>>,
    import_dialog: Option<Task<()>>,
}

impl Document {
    pub(super) fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title: SharedString = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
            .into();
        window.set_window_title(&title);
        window.set_document_path(Some(&path));
        platform::follow_appearance(window, cx);
        let header_available = path.extension().is_some_and(|ext| {
            ["csv", "tsv", "xlsx"]
                .iter()
                .any(|kind| ext.eq_ignore_ascii_case(kind))
        });
        let initial = Import {
            sheet: None,
            has_header: true,
        };
        let mut document = Self {
            path,
            title,
            table: None,
            sheets: Vec::new(),
            displayed: initial.clone(),
            header_available,
            load: Load::Ready,
            reader: None,
            import_dialog: None,
        };
        document.request(initial, window, cx);
        document
    }

    fn request(&mut self, import: Import, window: &mut Window, cx: &mut Context<Self>) {
        if self.table.is_some() && self.displayed == import {
            self.load = Load::Ready;
        } else {
            self.load = Load::Pending(import);
            self.start_pending(window, cx);
        }
        cx.notify();
    }

    // Disk reads are blocking. Keep one running reader and only the latest requested
    // import, so a cancelled read cannot install a stale table or overlap a new decode.
    fn start_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader.is_some() || !matches!(self.load, Load::Pending(_)) {
            return;
        }
        let path = self.path.clone();
        let list_sheets = self.sheets.is_empty()
            && path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("xlsx"));
        self.reader = Some(cx.spawn_in(window, async move |this, cx| {
            // Several clicks may arrive before GPUI schedules the reader.
            let Ok(Some(import)) = this.update(cx, |this, _| {
                if let Load::Pending(latest) = &this.load {
                    Some(latest.clone())
                } else {
                    this.reader = None;
                    None
                }
            }) else {
                return;
            };
            let options = import.clone();
            let (sheets, dataset) = cx
                .background_spawn(async move {
                    let sheets = if list_sheets {
                        match gewu_core::xlsx_sheet_names(&path) {
                            Ok(names) => Some(names),
                            Err(error) => return (None, Err(error)),
                        }
                    } else {
                        None
                    };
                    let sheet = options.sheet.or_else(|| sheets.as_ref()?.first().cloned());
                    let options = OpenOptions {
                        sheet,
                        has_header: options.has_header,
                        ..Default::default()
                    };
                    (sheets, Dataset::open_with_options(&path, &options))
                })
                .await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.reader = None;
                    if let Some(sheets) = sheets {
                        this.sheets = sheets;
                    }
                    if matches!(&this.load, Load::Pending(latest) if *latest == import) {
                        match dataset {
                            Ok(dataset) => this.install(dataset, import, window, cx),
                            Err(error) => this.load = Load::Failed(import, error),
                        }
                    } else {
                        this.start_pending(window, cx);
                    }
                    cx.notify();
                })
            });
        }));
    }

    fn install(
        &mut self,
        dataset: Dataset,
        mut import: Import,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let info = dataset.info();
        self.header_available =
            matches!(info.format, FileType::Csv | FileType::Tsv | FileType::Xlsx);
        if let FileMetadata::Xlsx { sheet, sheets } = info.metadata {
            self.sheets = sheets;
            import.sheet = Some(sheet);
        }
        self.displayed = import;
        let table = cx.new(|cx| {
            let mut table = TableView::new(TableData::from_dataset(Arc::new(dataset)), window, cx);
            table.document_title = Some(self.title.clone());
            table.delimited_header = matches!(info.format, FileType::Csv | FileType::Tsv)
                .then_some(self.displayed.has_header);
            if let Some(previous) = &self.table {
                table.inherit_document_state(previous, cx);
            } else {
                table.initialize_inspector(cx);
            }
            table
        });
        window.focus(&table.read(cx).focus.clone(), cx);
        if self.table.is_none() {
            record_recent(&self.path, cx);
        }
        self.table = Some(table);
        self.load = Load::Ready;
    }

    fn toggle_header(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request(
            Import {
                has_header: !self.displayed.has_header,
                ..self.displayed.clone()
            },
            window,
            cx,
        );
    }

    fn import_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_dialog.is_some() {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Info,
            "Import Options",
            Some("Choose how to read the first row of this file."),
            &["Use as Column Names", "Read as Data", "Cancel"],
            cx,
        );
        self.import_dialog = Some(cx.spawn_in(window, async move |this, cx| {
            let selected = answer.await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.import_dialog = None;
                    if let Ok(answer @ (0 | 1)) = selected {
                        this.request(
                            Import {
                                has_header: answer == 0,
                                ..this.displayed.clone()
                            },
                            window,
                            cx,
                        );
                    }
                })
            });
        }));
    }
}

impl Render for Document {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let mut view = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.foreground)
            .on_drop(|paths: &ExternalPaths, _, cx| open_paths(paths.paths().to_vec(), cx));
        if let Some(table) = &self.table {
            let table = table.clone();
            view = view.on_action(move |_: &GetInfo, _, cx| {
                info::open(table.read(cx).data.snapshot().dataset().info(), cx);
            });
        }
        if self.header_available {
            view = view.on_action(cx.listener(
                |doc, _: &crate::actions::ToggleImportHeader, w, cx| doc.toggle_header(w, cx),
            ));
        }
        if self.header_available && !matches!(self.load, Load::Pending(_)) {
            view = view.on_action(
                cx.listener(|this, _: &ImportOptions, w, cx| this.import_options(w, cx)),
            );
        }
        if let Some(table) = &self.table {
            view = view.child(div().flex_1().min_h_0().child(table.clone()));
        } else {
            view = view.child(TableView::document_titlebar(
                self.title.clone(),
                None,
                window,
                cx,
            ));
        }
        if !self.sheets.is_empty() || (self.header_available && self.table.is_none()) {
            let mut bar = div()
                .id("worksheet-options")
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .overflow_x_scroll()
                .bg(colors.surface)
                .border_t_1()
                .border_color(colors.border)
                .text_sm();
            for (index, name) in self.sheets.iter().enumerate() {
                let sheet = name.clone();
                let active = self.displayed.sheet.as_ref() == Some(name);
                bar = bar.child(
                    button(("sheet", index), name.clone(), active, cx)
                        .flex_none()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request(
                                Import {
                                    sheet: Some(sheet.clone()),
                                    ..this.displayed.clone()
                                },
                                window,
                                cx,
                            )
                        })),
                );
            }
            if self.header_available {
                let document = cx.weak_entity();
                bar = bar.child(
                    crate::panels::header_checkbox(self.displayed.has_header, cx).on_change(
                        move |_, _, w, cx| {
                            let _ = document.update(cx, |doc, cx| doc.toggle_header(w, cx));
                        },
                    ),
                );
            }
            view = view.child(bar);
        }
        match &self.load {
            Load::Ready => {}
            Load::Pending(_) => {
                view = view.child(div().flex_none().px_3().py_1().text_sm().child("Opening…"));
            }
            Load::Failed(import, error) => {
                let import = import.clone();
                view = view.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .text_sm()
                        .child(
                            button("retry-open", "Retry", false, cx)
                                .flex_none()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request(import.clone(), window, cx)
                                })),
                        )
                        .child(div().flex_1().min_w_0().child(format!(
                            "Could not open table: {}",
                            crate::errors::message(error)
                        ))),
                );
            }
        }
        view
    }
}

#[cfg(test)]
#[path = "../shell_tests.rs"]
mod tests;
