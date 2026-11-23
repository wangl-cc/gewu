use super::*;
use crate::table::Notice;

impl TableView {
    pub(in crate::table) fn status_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let rows = self.data.row_count();
        let columns = crate::counted(self.data.columns.len() as u64, "column");
        let row_count = if self.data.query().filters.is_empty() {
            crate::counted(rows, "row")
        } else {
            format!(
                "{rows} / {}",
                crate::counted(self.data.snapshot().dataset().n_rows() as u64, "row")
            )
        };
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
            .child(format!("{row_count} · {columns}"));
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
        for (filters, count, id, noun) in [
            (true, applied.filters.len(), "filtered-count", "filtered"),
            (false, applied.sorts.len(), "sorted-count", "sorted"),
        ] {
            if count == 0 {
                continue;
            }
            let open = matches!(
                (&self.controls.drawer, filters),
                (Some(Drawer::Filters), true) | (Some(Drawer::Sort { summary: true }), false)
            );
            let content = if open {
                Some(if filters {
                    self.filter_summary(window, cx)
                } else {
                    self.sort_panel(window, cx)
                })
            } else {
                None
            };
            bar = bar.child(
                base::Popover::new(("status-summary", filters as usize))
                    .anchor(Anchor::BottomRight)
                    .offset(px(4.))
                    .open(open)
                    .overlay_closable(self.controls.menu.is_none())
                    .trigger(button(id, format!("{count} {noun}"), false, cx).aria_expanded(open))
                    .on_open_change(cx.listener(move |t, open: &bool, w, cx| {
                        if *open {
                            t.controls.drawer = Some(if filters {
                                Drawer::Filters
                            } else {
                                Drawer::Sort { summary: true }
                            });
                        } else if matches!(
                            (&t.controls.drawer, filters),
                            (Some(Drawer::Filters), true)
                                | (Some(Drawer::Sort { summary: true }), false)
                        ) {
                            t.close_panel(w, cx);
                        }
                        cx.notify();
                    }))
                    .content(move |_, _, _| div().children(content)),
            );
        }
        self.export_status(bar, cx)
    }

    pub(super) fn filter_summary(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = self.drawer("Active filters", window, cx);
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
                super::footer()
                    .child(
                        button("clear-all-filters", "Clear all filters", false, cx).on_click(
                            cx.listener(|t, _, w, cx| {
                                t.close_panel(w, cx);
                                t.clear_filters(cx);
                            }),
                        ),
                    )
                    .child(self.close_button("Done", cx)),
            )
            .into_any_element()
    }
}
