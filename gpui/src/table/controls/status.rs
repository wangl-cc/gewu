use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        button::{Button as StyledButton, ButtonVariants},
    },
};

use super::*;
use crate::{messages::format_count, table::Notice};

impl TableView {
    pub(in crate::table) fn status_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = crate::theme::GewuPalette::get(cx);
        let rows = self.data.row_count();
        let columns = crate::counted(self.data.columns.len() as u64, "column");
        let row_count = if self.data.query().filters.is_empty() {
            crate::counted(rows, "row")
        } else {
            format!(
                "{} / {}",
                format_count(rows),
                crate::counted(self.data.snapshot().dataset().n_rows() as u64, "row")
            )
        };
        let worksheet_tabs = self.worksheet_tabs.clone();
        let bar = div()
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
            .font_features(crate::theme::tabular_digits());
        let mut status = div().min_w_0().flex().items_center().gap_2().child(
            div()
                .min_w_0()
                .truncate()
                .child(format!("{row_count} · {columns}")),
        );
        if let Some(selection) = self.selection {
            status = status.child(div().min_w_0().truncate().child(format!(
                "{} selected",
                format_count(selection.rows().end - selection.rows().start)
            )));
        }
        if self.data.queries().is_running() {
            status = status.child(div().min_w_0().truncate().child("Applying query…"));
        }
        if let Some(error) = self.data.queries().error() {
            status = status.child(
                crate::errors::inline_warning(crate::errors::message(error))
                    .truncate()
                    .text_color(colors.danger_text),
            );
        }
        match &self.notice {
            Some(Notice::SelectCellToFilter) if self.selection.is_none() => {
                status = status.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(crate::errors::SELECT_CELL_TO_FILTER),
                );
            }
            Some(Notice::CopyFailed(message) | Notice::FilterFailed(message)) => {
                status = status.child(
                    crate::errors::inline_warning(message.clone())
                        .id("operation-notice")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .text_color(colors.danger_text)
                        .min_w_0()
                        .truncate()
                        .test_support(),
                );
            }
            _ => {}
        }
        if worksheet_tabs.is_some() {
            // Keep a track for navigating tabs, alongside working summary controls at narrow
            // widths.
            status = status.max_w(relative(0.72)).justify_end();
        } else {
            status = status.flex_1().child(div().flex_1());
        }
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
            status = status.child(
                base::Popover::new(("status-summary", filters as usize))
                    .anchor(Anchor::BottomRight)
                    .offset(px(4.))
                    .open(open)
                    .overlay_closable(self.controls.menu.is_none())
                    .trigger(
                        button(
                            id,
                            format!("{} {noun}", format_count(count as u64)),
                            false,
                            cx,
                        )
                        .flex_none()
                        .aria_expanded(open),
                    )
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
        bar.children(worksheet_tabs.map(|tabs| div().flex_1().min_w_0().child(tabs)))
            .child(
                self.export_status(status, cx)
                    .id("footer-status")
                    .test_support(),
            )
    }

    pub(super) fn filter_summary(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = self.drawer("Active filters", window, cx);
        for entry in &self.data.query().filters {
            let id = entry.column;
            let Some(column) = self.data.columns.iter().find(|c| c.id == id) else {
                continue;
            };
            let raw = id.into_raw() as usize;
            let row =
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(
                        StyledButton::new(("locate-filter", raw))
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Locate))
                            .label(crate::data::preview_with_limit(&column.name, 36))
                            .accessibility_label(format!("Show column {}", column.name))
                            .tooltip("Show column")
                            .on_click(cx.listener(move |t, _, w, cx| {
                                t.commit_grid_layout(cx);
                                if let Some(column) = t.data.columns.position(id) {
                                    let row = t.selection.map(|s| s.head.row).unwrap_or(0);
                                    let cell = Cell {
                                        column: column.index(),
                                        row,
                                    };
                                    t.reveal(cell);
                                    if row < t.data.row_count() {
                                        t.set_selection(Some(Selection::single(cell)), cx);
                                    }
                                }
                                t.close_panel(w, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(button(("clear-filter", raw), "Clear", false, cx).on_click(
                        cx.listener(move |t, _, w, cx| {
                            t.close_panel(w, cx);
                            t.set_filter(id, None, cx);
                        }),
                    ));
            let detail = div()
                .pl(px(24.))
                .text_size(px(11.))
                .text_color(crate::theme::GewuPalette::get(cx).muted_foreground)
                .child(crate::panels::filter_description(&entry.filter));
            panel = panel.child(div().flex().flex_col().gap_0().child(row).child(detail));
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
