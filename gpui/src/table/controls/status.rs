use super::*;
use crate::table::Notice;

impl TableView {
    pub(in crate::table) fn status_bar(&self, cx: &mut Context<Self>) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut bar = div()
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
            .child(if self.data.query().filters.is_empty() {
                format!(
                    "{} rows · {} columns",
                    self.data.row_count(),
                    self.data.columns.len()
                )
            } else {
                format!(
                    "{} / {} rows · {} columns",
                    self.data.row_count(),
                    self.data.snapshot().dataset().n_rows(),
                    self.data.columns.len()
                )
            });
        if let Some(selection) = self.selection {
            bar = bar.child(format!(
                "{} selected",
                selection.rows().end - selection.rows().start
            ));
        }
        if self.data.queries().is_running() {
            bar = bar.child("Applying query…");
        }
        if let Some(error) = self.data.queries().error() {
            bar = bar.child(
                div()
                    .text_color(colors.destructive)
                    .child(crate::errors::message(error)),
            );
        }
        match &self.notice {
            Some(Notice::SelectCellToFilter) if self.selection.is_none() => {
                bar = bar.child(crate::errors::SELECT_CELL_TO_FILTER);
            }
            Some(Notice::CopyFailed(message) | Notice::FilterFailed(message)) => {
                bar = bar.child(
                    div()
                        .id("operation-notice")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .text_color(colors.destructive)
                        .child(message.clone())
                        .test_support(),
                );
            }
            _ => {}
        }
        bar = bar.child(div().flex_1());
        let applied = self.data.query();
        if !applied.filters.is_empty() {
            bar = bar.child(
                button(
                    "filtered-count",
                    format!("{} filtered", applied.filters.len()),
                    false,
                    cx,
                )
                .on_click(cx.listener(|t, ev: &ClickEvent, _, cx| {
                    t.controls.drawer = Some(Drawer::Filters(ev.position()));
                    cx.notify();
                })),
            );
        }
        if !applied.sorts.is_empty() {
            bar = bar.child(
                button(
                    "sorted-count",
                    format!("{} sorted", applied.sorts.len()),
                    false,
                    cx,
                )
                .on_click(cx.listener(|t, ev: &ClickEvent, _, cx| {
                    t.controls.drawer = Some(Drawer::Sort(Some(ev.position())));
                    cx.notify();
                })),
            );
        }
        if let Some(header) = self.delimited_header {
            let focus = self.focus.clone();
            bar = bar.child(crate::panels::header_checkbox(header, cx).on_change(
                move |_, _, w, cx| {
                    focus.dispatch_action(&crate::actions::ToggleImportHeader, w, cx);
                },
            ));
        }
        self.export_status(bar, cx)
    }

    pub(super) fn filter_summary(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = self.drawer("Active filters", cx);
        for entry in &self.data.query().filters {
            let id = entry.column;
            let Some(column) = self.data.columns.iter().find(|c| c.id == id) else {
                continue;
            };
            let raw = id.into_raw() as usize;
            panel = panel
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .items_center()
                        .child(
                            button(("locate-filter", raw), column.name.clone(), false, cx)
                                .on_click(cx.listener(move |t, _, w, cx| {
                                    if let Some(column) = t.data.columns.position(id) {
                                        let row = t.selection.map(|s| s.head.row).unwrap_or(0);
                                        let cell = Cell {
                                            column: column.index(),
                                            row,
                                        };
                                        t.reveal(cell);
                                        if row < t.data.row_count() {
                                            t.selection = Some(Selection::single(cell));
                                        }
                                    }
                                    t.close_panel(w, cx);
                                })),
                        )
                        .child(button(("clear-filter", raw), "Clear", false, cx).on_click(
                            cx.listener(move |t, _, w, cx| {
                                t.close_panel(w, cx);
                                t.set_filter(id, None, cx);
                            }),
                        )),
                )
                .child(crate::panels::filter_description(&entry.filter));
        }
        panel
            .child(
                button("clear-all-filters", "Clear all filters", false, cx).on_click(cx.listener(
                    |t, _, w, cx| {
                        t.close_panel(w, cx);
                        t.clear_filters(cx);
                    },
                )),
            )
            .into_any_element()
    }
}
