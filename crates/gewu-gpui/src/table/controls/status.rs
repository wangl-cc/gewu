//! The table's contributions to the document status bar: counts, query state, notices and
//! the applied-condition summaries with their popovers.
use gpui_kit::{assets::IconName, component::Icon};

use super::*;
use crate::{table::Notice, ui::messages::format_count};

impl TableView {
    /// Counts, query progress and operation notices for the document's status bar.
    pub(crate) fn status_items(&self, cx: &App) -> Vec<AnyElement> {
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
        let mut items = vec![
            div()
                .min_w_0()
                .truncate()
                .child(format!("{row_count} · {columns}"))
                .into_any_element(),
        ];
        if let Some(selection) = self.grid.selection() {
            items.push(
                div()
                    .min_w_0()
                    .truncate()
                    .child(format!(
                        "{} selected",
                        format_count(selection.rows().end - selection.rows().start)
                    ))
                    .into_any_element(),
            );
        }
        if self.data.queries().is_running() {
            items.push(
                div()
                    .min_w_0()
                    .truncate()
                    .child("Applying query…")
                    .into_any_element(),
            );
        }
        if let Some(error) = self.data.queries().error() {
            items.push(
                crate::ui::errors::inline_warning(crate::ui::errors::message(error))
                    .truncate()
                    .text_color(colors.danger_text)
                    .into_any_element(),
            );
        }
        match &self.notice {
            Some(Notice::SelectCellToFilter) if self.grid.selection().is_none() => {
                items.push(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(crate::ui::errors::SELECT_CELL_TO_FILTER)
                        .into_any_element(),
                );
            }
            Some(Notice::CopyFailed(message) | Notice::FilterFailed(message)) => {
                items.push(
                    crate::ui::errors::inline_warning(message.clone())
                        .id("operation-notice")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .text_color(colors.danger_text)
                        .min_w_0()
                        .truncate()
                        .test_support()
                        .into_any_element(),
                );
            }
            _ => {}
        }
        items
    }

    /// Status-bar triggers for the applied filter and sort summaries.
    pub(crate) fn condition_summaries(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let applied = self.data.query();
        let mut summaries = Vec::new();
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
            let label: SharedString = format!("{} {noun}", format_count(count as u64)).into();
            let trigger = crate::ui::status_button(id, label, open, cx).aria_expanded(open);
            summaries.push(
                base::Popover::new(("status-summary", filters as usize))
                    .anchor(Anchor::BottomRight)
                    .offset(px(4.))
                    .open(open)
                    .overlay_closable(self.controls.menu.is_none())
                    .trigger(trigger)
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
                    .content(move |_, _, _| div().children(content))
                    .into_any_element(),
            );
        }
        summaries
    }

    pub(super) fn filter_summary(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::theme::GewuPalette::get(cx);
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
                    .gap_2()
                    .items_center()
                    .child(
                        crate::ui::ghost_button(
                            ("locate-filter", raw),
                            format!("Show column {}", column.name),
                            false,
                            false,
                            cx,
                        )
                        .tooltip(crate::ui::tooltip("Show column"))
                        .min_w_0()
                        .gap(px(6.))
                        .mx(px(-4.))
                        .px(px(4.))
                        .h(px(24.))
                        .rounded(px(5.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(
                            Icon::new(IconName::Locate)
                                .size(px(13.))
                                .text_color(colors.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(crate::table::data::preview_with_limit(&column.name, 36)),
                        )
                        .on_click(cx.listener(move |t, _, w, cx| {
                            t.commit_grid_layout(cx);
                            if let Some(column) = t.data.columns.position(id) {
                                let row = t.grid.selection().map(|s| s.head.row).unwrap_or(0);
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
                .pl(px(19.))
                .text_size(px(11.))
                .text_color(colors.muted_foreground)
                .child(crate::panels::filter_description(&entry.filter));
            panel = panel.child(div().flex().flex_col().gap_0().child(row).child(detail));
        }
        if self.data.query().filters.len() > 1 {
            panel = panel.child(super::footer().child(
                button("clear-all-filters", "Clear all filters", false, cx).on_click(cx.listener(
                    |t, _, w, cx| {
                        t.close_panel(w, cx);
                        t.clear_filters(cx);
                    },
                )),
            ));
        }
        panel.into_any_element()
    }
}
