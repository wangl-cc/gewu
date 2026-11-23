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
    focus: FocusHandle,
    title: SharedString,
    table: Option<Entity<TableView>>,
    sheets: Vec<String>,
    displayed: Import,
    header_available: bool,
    load: Load,
    reader: Option<Task<()>>,
    import_dialog: Option<Task<()>>,
    show_error_details: bool,
}

impl Document {
    pub(super) fn new(
        path: PathBuf,
        has_header: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title: SharedString = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
            .into();
        window.set_window_title(&title);
        window.set_document_path(Some(&path));
        platform::follow_appearance(window, cx);
        let header_available = super::supports_header(&path);
        let initial = Import {
            sheet: None,
            has_header,
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut document = Self {
            path,
            focus,
            title,
            table: None,
            sheets: Vec::new(),
            displayed: initial.clone(),
            header_available,
            load: Load::Ready,
            reader: None,
            import_dialog: None,
            show_error_details: false,
        };
        document.request(initial, window, cx);
        document
    }

    fn import_selection(&self) -> &Import {
        match &self.load {
            Load::Pending(import) => import,
            Load::Failed(import, _) if self.table.is_none() => import,
            _ => &self.displayed,
        }
    }

    fn edit_import(
        &mut self,
        edit: impl FnOnce(&mut Import),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut import = self.import_selection().clone();
        edit(&mut import);
        self.request(import, window, cx);
    }

    fn set_load(&mut self, load: Load, cx: &mut Context<Self>) {
        self.load = load;
        self.show_error_details = false;
        cx.notify();
    }

    fn request(&mut self, import: Import, window: &mut Window, cx: &mut Context<Self>) {
        if self.table.is_some() && self.displayed == import {
            self.set_load(Load::Ready, cx);
        } else {
            self.set_load(Load::Pending(import), cx);
            self.start_pending(window, cx);
        }
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
                            Err(error) => this.set_load(Load::Failed(import, error), cx),
                        }
                    } else {
                        this.start_pending(window, cx);
                        cx.notify();
                    }
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
        self.set_load(Load::Ready, cx);
    }

    fn import_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_dialog.is_some() {
            return;
        }
        let has_conditions = self.table.as_ref().is_some_and(|table| {
            let data = &table.read(cx).data;
            [data.query().as_ref(), data.queries().requested()]
                .iter()
                .any(|query| !query.filters.is_empty() || !query.sorts.is_empty())
        });
        let row = if self.sheets.is_empty() {
            "first row"
        } else {
            "first occupied row"
        };
        let current = if self.import_selection().has_header {
            "column names"
        } else {
            "data"
        };
        let mut detail = format!("The {row} is currently read as {current}.");
        if has_conditions {
            detail.push_str(" Changing this option will clear Find, filters and sorting.");
        }
        let answer = window.prompt(
            PromptLevel::Info,
            "Import Options",
            Some(&detail),
            &["Use as Column Names", "Read as Data", "Cancel"],
            cx,
        );
        self.import_dialog = Some(cx.spawn_in(window, async move |this, cx| {
            let selected = answer.await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.import_dialog = None;
                    if let Ok(answer @ (0 | 1)) = selected {
                        this.edit_import(|import| import.has_header = answer == 0, window, cx);
                    }
                })
            });
        }));
    }

    fn failed_open(
        &self,
        import: &Import,
        error: &gewu_core::Error,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = Theme::global(cx).tokens.colors;
        let import = import.clone();
        let retry = button("retry-open", "Retry", false, cx).on_click(
            cx.listener(move |this, _, window, cx| this.request(import.clone(), window, cx)),
        );
        // A failed replacement preserves the table and worksheet controls.
        if self.table.is_some() {
            return div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .text_sm()
                .child(retry)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(crate::errors::message(error)),
                )
                .into_any_element();
        }
        let reason = crate::errors::open_reason(error);
        let mut content = div()
            .flex()
            .flex_col()
            .items_start()
            .gap_3()
            .px_4()
            .w_full()
            .max_w(px(475.))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Could not open this file"),
            )
            .child(
                div()
                    .id("open-error-reason")
                    .role(Role::Alert)
                    .aria_label(reason.clone())
                    .text_color(colors.muted_foreground)
                    .child(reason),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button("open-another-file", "Open Another File…", true, cx)
                            .on_click(|_, _, cx| super::choose_files(cx)),
                    )
                    .child(retry)
                    .child(
                        button("reveal-failed-file", "Show in Finder", false, cx)
                            .on_click(cx.listener(|doc, _, _, cx| cx.reveal_path(&doc.path))),
                    ),
            )
            .child(
                button("open-error-details", "Technical details", false, cx)
                    .aria_expanded(self.show_error_details)
                    .on_click(cx.listener(|doc, _, _, cx| {
                        doc.show_error_details = !doc.show_error_details;
                        cx.notify();
                    })),
            );
        if self.show_error_details {
            let details = error.to_string();
            content = content.child(
                div()
                    .id("open-error-text")
                    .aria_label(details.clone())
                    .max_h(px(135.))
                    .w_full()
                    .overflow_y_scroll()
                    .text_sm()
                    .test_support()
                    .child(details),
            );
        }
        div()
            .id("open-error")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .py_4()
            .child(content)
            .into_any_element()
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
        } else {
            view = view.track_focus(&self.focus);
        }
        if self.header_available {
            view = view.on_action(
                cx.listener(|this, _: &ImportOptions, w, cx| this.import_options(w, cx)),
            );
        }
        if let Some(table) = &self.table {
            view = view.child(div().flex_1().min_h_0().child(table.clone()));
        } else {
            view = view.child(TableView::document_titlebar(self.title.clone(), window, cx));
        }
        if !self.sheets.is_empty() {
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
                let active = self.import_selection().sheet.as_ref() == Some(name);
                bar = bar.child(
                    button(("sheet", index), name.clone(), active, cx)
                        .flex_none()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_import(
                                |import| import.sheet = Some(sheet.clone()),
                                window,
                                cx,
                            )
                        })),
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
                view = view.child(self.failed_open(import, error, cx));
            }
        }
        view
    }
}

#[cfg(test)]
#[path = "../shell_tests.rs"]
mod tests;
