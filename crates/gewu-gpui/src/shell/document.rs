//! A document window: one file's import requests and its replaceable table, framed by the
//! state that outlives a replacement — the title bar, Inspector, export job, worksheet tabs
//! and status bar. Document-wide commands are handled here and forwarded to the table.
use std::{path::PathBuf, sync::Arc};

use gewu_core::{ColumnId, Dataset, FileMetadata, FileType, OpenOptions, SourceRow};
use gpui_kit::{
    base::{self, Tab, Theme},
    prelude::FluentBuilder,
    *,
};

use super::{
    GetInfo, ImportOptions,
    export::ExportController,
    import::{Import, ImportSession, Load, ReadOutcome},
    info, open_paths, record_recent,
};
use crate::{
    actions,
    panels::{Inspector, InspectorPage},
    platform,
    preferences::Preferences,
    table::{TableEvent, TableView, data::TableData},
    ui::{button, disclosure, segment, segmented_tabs},
};

/// A scatter point revealed in the table, marked in the chart until the selection leaves it.
struct MarkedPoint {
    chart: EntityId,
    column: ColumnId,
    row: SourceRow,
}

pub(crate) struct Document {
    path: PathBuf,
    focus: FocusHandle,
    title: SharedString,
    table: Option<Entity<TableView>>,
    _table_events: Option<Subscription>,
    inspector: Option<Entity<Inspector>>,
    marked_point: Option<MarkedPoint>,
    export: Option<Entity<ExportController>>,
    sheets: Vec<String>,
    worksheet_tabs: Option<Entity<WorksheetTabs>>,
    header_available: bool,
    import: ImportSession,
    reader: Option<Task<()>>,
    import_dialog: Option<Task<()>>,
    show_error_details: bool,
}

impl Document {
    pub(crate) fn new(
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
            _table_events: None,
            inspector: None,
            marked_point: None,
            export: None,
            sheets: Vec::new(),
            worksheet_tabs: None,
            header_available,
            import: ImportSession::new(initial),
            reader: None,
            import_dialog: None,
            show_error_details: false,
        };
        document.start_reader(window, cx);
        document
    }

    pub(crate) fn table(&self) -> Option<Entity<TableView>> {
        self.table.clone()
    }

    fn edit_import(
        &mut self,
        edit: impl FnOnce(&mut Import),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut import = self.import.selection().clone();
        edit(&mut import);
        self.request(import, window, cx);
    }

    /// The load state changed: technical details of an earlier failure collapse.
    fn load_changed(&mut self, cx: &mut Context<Self>) {
        self.show_error_details = false;
        cx.notify();
    }

    fn set_sheets(&mut self, sheets: Vec<String>, cx: &mut Context<Self>) {
        self.sheets = sheets;
        if !self.sheets.is_empty() && self.worksheet_tabs.is_none() {
            let document = cx.entity();
            self.worksheet_tabs = Some(cx.new(|cx: &mut Context<WorksheetTabs>| WorksheetTabs {
                document: document.downgrade(),
                scroll: ScrollHandle::new(),
                _changes: cx.observe(&document, |_, _, cx| cx.notify()),
            }));
        }
    }

    fn request(&mut self, import: Import, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.import.request(import);
        self.load_changed(cx);
        if start {
            self.start_reader(window, cx);
        }
    }

    /// Read the latest requested import in the background. See [`ImportSession`].
    fn start_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let list_sheets = self.sheets.is_empty()
            && FileType::from_path(&path).is_some_and(FileType::has_worksheets);
        self.reader = Some(cx.spawn_in(window, async move |this, cx| {
            // Several clicks may arrive before GPUI schedules the reader.
            let Ok(Some(import)) = this.update(cx, |this, _| this.import.begin_read()) else {
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
                        this.set_sheets(sheets, cx);
                    }
                    match this.import.read_finished(import.clone(), dataset) {
                        ReadOutcome::Install(dataset) => this.install(dataset, import, window, cx),
                        ReadOutcome::Failed => this.load_changed(cx),
                        ReadOutcome::Next => this.start_reader(window, cx),
                        ReadOutcome::Withdrawn => {}
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
        self.header_available = dataset.format().has_header_choice();
        if let FileMetadata::Xlsx { sheet, sheets } = dataset.metadata() {
            self.set_sheets(sheets.clone(), cx);
            import.sheet = Some(sheet.clone());
        }
        self.import.installed(import);
        let table =
            cx.new(|cx| TableView::new(TableData::from_dataset(Arc::new(dataset)), window, cx));
        self._table_events = Some(cx.subscribe(&table, Self::on_table_event));
        self.marked_point = None;
        let (snapshot, columns) = {
            let table = table.read(cx);
            (table.snapshot(), table.visible_columns())
        };
        if let Some(panel) = &self.inspector {
            panel.update(cx, |panel, cx| panel.set_dataset(snapshot, columns, cx));
        } else if self.table.is_none() && Preferences::get(cx).show_inspector {
            self.inspector = Some(cx.new(|cx| Inspector::new(snapshot, columns, cx)));
        }
        window.focus(&table.read(cx).focus.clone(), cx);
        if self.table.is_none() {
            record_recent(&self.path, cx);
        }
        self.table = Some(table);
        self.load_changed(cx);
    }

    fn on_table_event(
        &mut self,
        table: Entity<TableView>,
        event: &TableEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::ContextChanged => {
                if let Some(panel) = &self.inspector {
                    let (snapshot, columns) = {
                        let table = table.read(cx);
                        (table.snapshot(), table.visible_columns())
                    };
                    panel.update(cx, |panel, cx| panel.set_context(snapshot, columns, cx));
                }
            }
            TableEvent::SelectionChanged => {
                let Some(mark) = &self.marked_point else {
                    return;
                };
                // Reordering visible columns moves the cell's coordinates but keeps its identity.
                if table.read(cx).selected_source_cell() == Some((mark.column, mark.row)) {
                    return;
                }
                let chart = mark.chart;
                self.marked_point = None;
                if let Some(panel) = &self.inspector {
                    panel.update(cx, |panel, cx| panel.mark_scatter(chart, None, cx));
                }
            }
            TableEvent::Analyze(column) => self.analyze_column(*column, cx),
            TableEvent::Export(draft) => {
                let export = self
                    .export
                    .get_or_insert_with(|| cx.new(|_| ExportController::Idle))
                    .clone();
                export.update(cx, |export, cx| export.choose_path(draft.clone(), cx));
            }
        }
    }

    fn reveal_scatter_point(
        &mut self,
        hit: &actions::RevealScatterPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(table), Some(panel)) = (self.table.clone(), self.inspector.clone()) else {
            return;
        };
        let Some(column) = panel.read(cx).plotted_column(hit.chart) else {
            return;
        };
        if !table.update(cx, |table, cx| {
            table.reveal_source_cell(hit.source_row, column, window, cx)
        }) {
            return;
        }
        self.marked_point = Some(MarkedPoint {
            chart: hit.chart,
            column,
            row: hit.source_row,
        });
        panel.update(cx, |panel, cx| {
            panel.mark_scatter(hit.chart, Some(hit.source_row), cx)
        });
    }

    fn can_export(&self, table: &TableView, cx: &App) -> bool {
        table.can_export()
            && self
                .export
                .as_ref()
                .is_none_or(|export| !export.read(cx).is_busy())
    }

    fn inspector_visible(&self, cx: &App) -> bool {
        self.inspector
            .as_ref()
            .is_some_and(|panel| panel.read(cx).is_visible())
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.set_inspector_visible(!self.inspector_visible(cx), cx);
    }

    fn set_inspector_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let Some(table) = self.table.clone() else {
            return;
        };
        table.update(cx, |table, cx| table.dismiss_panels(cx));
        if visible && self.inspector.is_none() {
            let (snapshot, columns) = {
                let table = table.read(cx);
                (table.snapshot(), table.visible_columns())
            };
            self.inspector = Some(cx.new(|cx| Inspector::new(snapshot, columns, cx)));
        }
        if let Some(panel) = &self.inspector {
            panel.update(cx, |panel, cx| panel.set_visible(visible, cx));
        }
        Preferences::update(cx, |p| p.show_inspector = visible);
        cx.notify();
    }

    fn show_inspector_page(&mut self, page: InspectorPage, cx: &mut Context<Self>) {
        self.set_inspector_visible(true, cx);
        if let Some(panel) = &self.inspector {
            panel.update(cx, |panel, cx| panel.set_page(page, cx));
        }
    }

    fn analyze_column(&mut self, column: ColumnId, cx: &mut Context<Self>) {
        self.set_inspector_visible(true, cx);
        if let Some(panel) = &self.inspector {
            panel.update(cx, |panel, cx| panel.inspect(column, cx));
        }
    }

    fn import_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_dialog.is_some() {
            return;
        }
        let has_conditions = self
            .table
            .as_ref()
            .is_some_and(|table| table.read(cx).has_conditions());
        let row = if self.sheets.is_empty() {
            "first row"
        } else {
            "first occupied row"
        };
        let current = if self.import.selection().has_header {
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
        let colors = crate::theme::GewuPalette::get(cx);
        let import = import.clone();
        let retry =
            cx.listener(move |this, _, window, cx| this.request(import.clone(), window, cx));
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
                .child(crate::ui::errors::warning_icon())
                .child(button("retry-open", "Retry", false, cx).on_click(retry))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(crate::ui::errors::message(error)),
                )
                .into_any_element();
        }
        let reason = crate::ui::errors::open_reason(error);
        let recovery_button =
            |id, label, primary| button(id, label, primary, cx).px_3().flex().items_center();
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
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(crate::ui::errors::warning_icon())
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
                        recovery_button("open-another-file", "Open Another File…", true)
                            .on_click(|_, _, cx| super::choose_files(cx)),
                    )
                    .child(recovery_button("retry-open", "Retry", false).on_click(retry))
                    .child(
                        recovery_button("reveal-failed-file", "Show in Finder", false)
                            .on_click(cx.listener(|doc, _, _, cx| cx.reveal_path(&doc.path))),
                    ),
            )
            .child(
                disclosure(
                    "open-error-details",
                    "Technical details",
                    self.show_error_details,
                    cx,
                )
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
                    .p(px(12.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .font_family(Theme::global(cx).tokens.typography.mono.clone())
                    .text_size(px(12.))
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

impl Document {
    /// Document-wide commands. Each is available only while the table can perform it, so
    /// menus and shortcuts follow the focused document's state.
    fn table_actions(&self, view: Div, table: &Entity<TableView>, cx: &mut Context<Self>) -> Div {
        let state = table.read(cx);
        let clear_filters = state.has_requested_filters();
        let step_find = state.can_step_find(cx);
        let selected = state.selected_column();
        let analyzable = state.analyzable_column();
        let export = self.can_export(state, cx);
        let (info_table, escape_table) = (table.clone(), table.clone());
        let mut view = view
            .on_action(cx.listener(Self::reveal_scatter_point))
            .on_action(
                cx.listener(|doc, _: &actions::ShowInspector, _, cx| doc.toggle_inspector(cx)),
            )
            .on_action(cx.listener(|doc, _: &actions::ShowChartInspector, _, cx| {
                doc.show_inspector_page(InspectorPage::Chart, cx)
            }))
            .on_action(|_: &actions::ToggleRowNumbers, _, cx| {
                Preferences::update(cx, |p| p.show_row_numbers = !p.show_row_numbers);
            })
            .on_action(move |_: &GetInfo, _, cx| {
                info::open(info_table.read(cx).snapshot().dataset(), cx);
            })
            .on_action(move |_: &base::input::Escape, window, cx| {
                if !escape_table.update(cx, |table, cx| table.dismiss(window, cx)) {
                    cx.propagate();
                }
            });
        view = forward::<base::input::Search>(view, table, TableView::open_find);
        view = forward::<actions::ManageColumns>(view, table, TableView::open_columns);
        view = forward::<actions::EditSort>(view, table, TableView::open_sort);
        if clear_filters {
            view = forward::<actions::ClearFilters>(view, table, |table, _, cx| {
                table.clear_filters(cx)
            });
        }
        if step_find {
            view = forward::<actions::FindNext>(view, table, |table, _, cx| {
                table.step_find(false, cx)
            });
            view = forward::<actions::FindPrevious>(view, table, |table, _, cx| {
                table.step_find(true, cx)
            });
        }
        if let Some(column) = selected {
            view = forward::<actions::EditFilter>(view, table, move |table, window, cx| {
                table.filter_column(column, window, cx)
            });
        }
        if let Some(column) = analyzable {
            view = view.on_action(cx.listener(
                move |doc, _: &actions::AnalyzeSelectedColumn, _, cx| {
                    doc.analyze_column(column, cx)
                },
            ));
        }
        if export {
            view = forward::<actions::ExportTable>(view, table, TableView::open_export);
        }
        view
    }

    fn titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let bar = titlebar(self.title.clone(), window, cx);
        let Some(table) = &self.table else {
            return bar;
        };
        let colors = crate::theme::GewuPalette::get(cx);
        let inspector_visible = self.inspector_visible(cx);
        bar.child(table.read(cx).find_bar().clone()).child(
            crate::ui::ghost_button("show-inspector", "Inspector", inspector_visible, false, cx)
                .aria_expanded(inspector_visible)
                .size(px(28.))
                .rounded(px(6.))
                .child(
                    div()
                        .w(px(18.))
                        .h(px(14.))
                        .flex()
                        .justify_end()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(colors.foreground)
                        .child(div().mr(px(4.)).w(px(1.)).h_full().bg(colors.foreground)),
                )
                .on_click(cx.listener(|doc, _, _, cx| doc.toggle_inspector(cx))),
        )
    }

    /// The table between the title bar and the status bar, beside the visible Inspector.
    fn body(&self, table: &Entity<TableView>, cx: &App) -> Div {
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(div().flex_1().min_w_0().min_h_0().child(table.clone()))
            .children(
                self.inspector
                    .clone()
                    .filter(|panel| panel.read(cx).is_visible()),
            )
    }

    fn status_bar(
        &self,
        table: &Entity<TableView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = crate::theme::GewuPalette::get(cx);
        let (items, summaries) = table.update(cx, |table, cx| {
            (
                table.status_items(cx),
                table.condition_summaries(window, cx),
            )
        });
        let tabs = self.worksheet_tabs.clone();
        let status = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .children(items)
            .map(|status| {
                if tabs.is_some() {
                    // Keep a track for navigating tabs, alongside working summary controls at
                    // narrow widths.
                    status.max_w(relative(0.72)).justify_end()
                } else {
                    status.flex_1().child(div().flex_1())
                }
            })
            .children(summaries)
            .children(self.export.clone())
            .id("footer-status")
            .test_support();
        div()
            .id("document-footer")
            .test_support()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .min_h(px(24.))
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .text_color(colors.muted_foreground)
            .text_size(px(11.))
            .font_features(crate::theme::tabular_digits())
            .children(tabs.map(|tabs| div().flex_1().min_w_0().child(tabs)))
            .child(status)
    }
}

impl Render for Document {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = crate::theme::GewuPalette::get(cx);
        let mut view = div()
            .key_context("GewuDocument")
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .text_color(colors.foreground)
            .on_drop(|paths: &ExternalPaths, _, cx| open_paths(paths.paths().to_vec(), cx));
        if self.header_available {
            view = view.on_action(
                cx.listener(|this, _: &ImportOptions, w, cx| this.import_options(w, cx)),
            );
        }
        let load = match self.import.load() {
            Load::Ready => None,
            Load::Pending(_) => Some(
                div()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child("Opening…")
                    .into_any_element(),
            ),
            Load::Failed(import, error) => Some(self.failed_open(import, error, cx)),
        };
        let view = view.child(self.titlebar(window, cx));
        if let Some(table) = self.table.clone() {
            return self
                .table_actions(view, &table, cx)
                .child(self.body(&table, cx))
                .child(self.status_bar(&table, window, cx))
                .children(load);
        }
        // A workbook with an unreadable first sheet still needs a route to its other sheets.
        let tabs = self.worksheet_tabs.clone().map(|tabs| {
            div()
                .flex_none()
                .px_2()
                .bg(colors.surface)
                .border_t_1()
                .border_color(colors.border)
                .child(tabs)
        });
        view.track_focus(&self.focus).children(load).children(tabs)
    }
}

/// Run a document command on the table.
fn forward<A: Action>(
    view: Div,
    table: &Entity<TableView>,
    run: impl Fn(&mut TableView, &mut Window, &mut Context<TableView>) + 'static,
) -> Div {
    let table = table.clone();
    view.on_action(move |_: &A, window, cx| table.update(cx, |table, cx| run(table, window, cx)))
}

/// The window's title row: the file name, draggable like a native title bar.
fn titlebar(name: SharedString, window: &mut Window, cx: &mut App) -> Div {
    let colors = crate::theme::GewuPalette::get(cx);
    let title = div()
        .id("document-title")
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .truncate()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(name),
        );
    #[cfg(target_os = "macos")]
    let title = {
        let pressed = window.use_state(cx, |_, _| false);
        title
            .pl(if window.is_fullscreen() {
                px(12.)
            } else {
                px(90.)
            })
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&pressed, |pressed, ev: &MouseDownEvent, w, _| {
                    *pressed = ev.click_count == 1;
                    if ev.click_count == 2 {
                        w.titlebar_double_click();
                    }
                }),
            )
            .on_mouse_down_out(window.listener_for(&pressed, |pressed, _, _, _| *pressed = false))
            .on_mouse_move(
                window.listener_for(&pressed, |pressed, ev: &MouseMoveEvent, w, _| {
                    if std::mem::take(pressed) && ev.pressed_button == Some(MouseButton::Left) {
                        w.start_window_move();
                    }
                }),
            )
    };
    #[cfg(not(target_os = "macos"))]
    let title = {
        let _ = window;
        title.pl_3()
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .h(px(52.))
        .pr_3()
        .bg(colors.surface)
        .border_b_1()
        .border_color(colors.border)
        .text_sm()
        .child(title.test_support())
}

struct WorksheetTabs {
    document: WeakEntity<Document>,
    scroll: ScrollHandle,
    _changes: Subscription,
}

impl Render for WorksheetTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut scroller = div()
            .id("worksheet-options")
            .flex()
            .min_w_0()
            .py(px(2.))
            .test_support();
        if let Some(document) = self.document.upgrade() {
            let mut tabs = segmented_tabs("worksheet-tabs", "Worksheets", window, cx)
                .flex_shrink_1()
                .min_w_0()
                .overflow_x_scroll()
                .track_scroll(&self.scroll);
            let doc = document.read(cx);
            for (index, name) in doc.sheets.iter().enumerate() {
                let sheet = name.clone();
                let parent = self.document.clone();
                let scroll = self.scroll.clone();
                let active = doc.import.selection().sheet.as_ref() == Some(name);
                let tab = Tab::new(("sheet", index))
                    .selected(active)
                    .set_position(index + 1, doc.sheets.len())
                    .accessibility_label(name.clone())
                    .on_click(move |_, window, cx| {
                        scroll.scroll_to_item(index);
                        let _ = parent.update(cx, |doc, cx| {
                            doc.edit_import(
                                |import| import.sheet = Some(sheet.clone()),
                                window,
                                cx,
                            );
                        });
                    });
                tabs = tabs.child(segment(tab, name.clone(), active, cx).flex_none().px_2());
            }
            scroller = scroller.child(tabs);
        }
        scroller
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
