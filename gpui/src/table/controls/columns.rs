use gewu_core::ColumnId;
use gpui_kit::{
    base::input::{Input, InputAccessibility, InputEvent, InputState},
    component::{Icon, IconName},
    prelude::FluentBuilder,
    *,
};

use super::{Cell, ColumnDrag, Drawer, Order, Preferences, Selection, TableView, button};
use crate::{columns::TableColumns, data::TableQuery};

fn uses_column(query: &TableQuery, id: ColumnId) -> bool {
    query.filters.iter().any(|filter| filter.column == id)
        || query.sorts.iter().any(|sort| sort.column == id)
}

#[derive(Default)]
pub(super) struct ColumnListScroll {
    scroll: ScrollHandle,
    edge: base::AutoScroll,
}

pub(super) struct HideConfirmation {
    ids: Vec<ColumnId>,
    skip_future: bool,
}

impl TableView {
    pub(in crate::table) fn open_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Find column");
            input.set_accessibility(
                InputAccessibility::new(Some(Role::SearchInput))
                    .with_label(Some("Find column".into())),
            );
            input
        });
        let events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.focus_panel(window, cx);
        self.controls.menu = None;
        self.controls.drawer = Some(Drawer::Columns {
            search,
            _events: events,
            confirm: None,
            list: ColumnListScroll::default(),
        });
        cx.notify();
    }

    pub(super) fn columns_panel(
        &self,
        search: &Entity<InputState>,
        confirm: Option<&HideConfirmation>,
        list: &ColumnListScroll,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = crate::theme::GewuPalette::get(cx);
        if let Some(confirmation) = confirm {
            return self.hide_confirmation(confirmation, window, cx);
        }
        let mut panel = self.drawer("Visible columns", window, cx);
        let query = search.read(cx).value().to_lowercase();
        let mut matches = Vec::new();
        for (position, column) in self.data.columns.visible_then_hidden() {
            if query.is_empty() || column.name.to_lowercase().contains(&query) {
                matches.push((position, column));
            }
        }
        let matching: Vec<_> = matches.iter().map(|(_, column)| column.id).collect();
        let to_show = matching.clone();
        let all_shown = matches.iter().all(|(position, _)| position.is_some());
        let all_hidden = matches.iter().all(|(position, _)| position.is_none());
        let (show_id, show_label) = if query.is_empty() {
            ("show-all-columns", "Show all")
        } else {
            ("show-matching", "Show matching")
        };
        let hide_label = if query.is_empty() {
            "Hide all"
        } else {
            "Hide matching"
        };
        let show_disabled = to_show.is_empty() || all_shown;
        let hide_disabled = matching.is_empty() || all_hidden;
        panel = panel.child(
            crate::panels::input_frame(div(), true, &search.focus_handle(cx), window, cx)
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .child(
                    Icon::new(IconName::Search)
                        .size(px(14.))
                        .text_color(colors.muted_foreground),
                )
                .child(div().min_w_0().flex_1().child(Input::new(search))),
        );
        let actions = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px_2()
            .py(px(5.))
            .bg(colors.surface)
            .border_b_1()
            .border_color(colors.border)
            .child(
                crate::panels::compact_button(show_id, show_label, show_disabled, cx)
                    .on_click(cx.listener(move |t, _, _, cx| t.set_visible(&to_show, true, cx))),
            )
            .child(
                crate::panels::compact_button("hide-matching", hide_label, hide_disabled, cx)
                    .on_click(cx.listener(move |t, _, w, cx| t.request_hide(&matching, w, cx))),
            );
        let mut rows = div()
            .id("column-list")
            .test_support()
            .track_scroll(&list.scroll)
            .on_drag_move(cx.listener(Self::scroll_column_drag))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.stop_column_scroll()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.stop_column_scroll()),
            )
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for (position, column) in matches {
            let id = column.id;
            let raw = id.into_raw() as usize;
            let mut row = div()
                .id(("column-rule", raw))
                .flex()
                .items_center()
                .h(px(29.))
                .px_2()
                .flex_none()
                .gap_2()
                .hover(move |r| r.bg(colors.button_secondary_hover))
                .child(
                    div()
                        .id(("column-grip", raw))
                        .test_support()
                        .w(px(10.))
                        .text_color(colors.muted_foreground)
                        .when(position.is_some(), |g| g.cursor(CursorStyle::OpenHand))
                        .child(if position.is_some() { "⠿" } else { "" }),
                )
                .child(
                    crate::panels::checkbox(
                        ("toggle-column", raw),
                        column.name.clone(),
                        position.is_some(),
                        cx,
                        cx.listener(move |t, _, w, cx| {
                            if t.data.columns.position(id).is_some() {
                                t.request_hide(&[id], w, cx);
                            } else {
                                t.set_visible(&[id], true, cx);
                            }
                        }),
                    )
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden(),
                )
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .truncate()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(column.display_type.clone()),
                );
            if let Some(position) = position {
                let index = position.index();
                let payload = ColumnDrag {
                    document: cx.entity_id(),
                    order: Order::Columns,
                    index,
                    id,
                    name: column.name.clone().into(),
                };
                row = row
                    .on_drag(payload.clone(), |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<ColumnDrag>(move |style, drag, _, _| {
                        payload.insertion_indicator(style, drag, colors.accent)
                    })
                    .on_drop(cx.listener(move |t, drag: &ColumnDrag, _, cx| {
                        if drag.document != cx.entity_id() || drag.order != Order::Columns {
                            return;
                        }
                        t.stop_column_scroll();
                        if let Some(from) = t.data.columns.position(drag.id)
                            && let Some(to) = t.data.columns.position(id)
                        {
                            t.change_columns(
                                true,
                                |columns| columns.move_to(from.index(), to.index()),
                                cx,
                            );
                            cx.notify();
                        }
                    }));
                row = row
                    .child(
                        super::icon_button(("column-earlier", raw), "Move column earlier", "↑", cx)
                            .disabled(index == 0)
                            .on_click(cx.listener(move |t, _, _, cx| t.move_column(id, -1, cx))),
                    )
                    .child(
                        super::icon_button(("column-later", raw), "Move column later", "↓", cx)
                            .disabled(index + 1 == self.data.columns.len())
                            .on_click(cx.listener(move |t, _, _, cx| t.move_column(id, 1, cx))),
                    );
            } else {
                row = row
                    .children((0..2).map(|_| div().size(px(super::ICON_BUTTON_SIZE)).flex_none()));
            }
            rows = rows.child(row);
        }
        panel
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.canvas)
                    .child(actions)
                    .child(rows),
            )
            .child(
                super::footer()
                    .flex_wrap()
                    .child(
                        button("restore-column-order", "Restore default order", false, cx)
                            .disabled(self.data.columns.is_in_source_order())
                            .on_click(cx.listener(|t, _, _, cx| {
                                if t.change_columns(true, TableColumns::restore_source_order, cx) {
                                    cx.notify();
                                }
                            })),
                    )
                    .child(self.close_button("Done", cx)),
            )
            .into_any_element()
    }

    fn hide_confirmation(
        &self,
        confirmation: &HideConfirmation,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let names = self
            .data
            .columns
            .iter()
            .filter(|c| confirmation.ids.contains(&c.id))
            .map(|c| format!("“{}”", c.name))
            .collect::<Vec<_>>();
        let conditions = [self.requested_query(), self.data.query().as_ref()];
        let sorted = conditions
            .iter()
            .any(|q| q.sorts.iter().any(|s| confirmation.ids.contains(&s.column)));
        let filtered = conditions.iter().any(|q| {
            q.filters
                .iter()
                .any(|f| confirmation.ids.contains(&f.column))
        });
        let removed = match (sorted, filtered) {
            (true, true) => "filtering and sorting",
            (true, false) => "sorting",
            _ => "filtering",
        };
        let (title, message) = if let [name] = names.as_slice() {
            (
                format!("Hide {name}?"),
                format!("This removes {removed} on this column."),
            )
        } else {
            (
                format!("Hide {}?", crate::counted(names.len() as u64, "column")),
                format!("This removes {removed} on {}.", names.join(", ")),
            )
        };
        self.drawer(title, window, cx)
            .child(
                crate::errors::inline_warning(message.clone())
                    .id("hide-explanation")
                    .role(Role::Label)
                    .aria_label(message)
                    .test_support(),
            )
            .child(crate::panels::checkbox(
                "skip-hide-warning",
                "Don’t ask again",
                confirmation.skip_future,
                cx,
                cx.listener(|t, _, _, cx| {
                    if let Some(Drawer::Columns {
                        confirm: Some(confirm),
                        ..
                    }) = &mut t.controls.drawer
                    {
                        confirm.skip_future = !confirm.skip_future;
                    }
                    cx.notify();
                }),
            ))
            .child(
                super::footer()
                    .child(
                        button("cancel-hide", "Cancel", false, cx)
                            .on_click(cx.listener(|t, _, w, cx| t.cancel_hide(w, cx))),
                    )
                    .child(
                        crate::panels::danger_button("confirm-hide", "Hide", cx)
                            .on_click(cx.listener(|t, _, w, cx| t.confirm_hide(w, cx))),
                    ),
            )
            .into_any_element()
    }

    fn scroll_column_drag(
        &mut self,
        event: &DragMoveEvent<ColumnDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        if drag.document != cx.entity_id() || drag.order != Order::Columns {
            return;
        }
        if let Some(Drawer::Columns { list, .. }) = &mut self.controls.drawer {
            let delta = event
                .bounds
                .contains(&event.event.position)
                .then(|| base::AutoScroll::compute_delta(event.event.position.y, event.bounds))
                .flatten();
            list.edge.set(delta, cx, |delta, table, cx| {
                if let Some(Drawer::Columns { list, .. }) = &mut table.controls.drawer {
                    let mut offset = list.scroll.offset();
                    let next = (offset.y - delta).clamp(-list.scroll.max_offset().y, px(0.));
                    if offset.y == next {
                        list.edge.stop();
                    } else {
                        offset.y = next;
                        list.scroll.set_offset(offset);
                        cx.notify();
                    }
                }
            });
        }
    }

    fn stop_column_scroll(&mut self) {
        if let Some(Drawer::Columns { list, .. }) = &mut self.controls.drawer {
            list.edge.stop();
        }
    }

    pub(super) fn cancel_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Drawer::Columns { confirm, .. }) = &mut self.controls.drawer {
            *confirm = None;
        }
        self.focus_panel(window, cx);
        cx.notify();
    }

    pub(super) fn confirm_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Drawer::Columns { confirm, .. }) = &mut self.controls.drawer else {
            return;
        };
        let Some(confirmation) = confirm.take() else {
            return;
        };
        if confirmation.skip_future {
            Preferences::update(cx, |p| p.skip_hide_warning = true);
        }
        self.set_visible(&confirmation.ids, false, cx);
        self.focus_panel(window, cx);
        cx.notify();
    }

    pub(super) fn request_hide(
        &mut self,
        ids: &[ColumnId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.hiding_clears_conditions(ids) && !Preferences::get(cx).skip_hide_warning {
            if let Some(Drawer::Columns { confirm, .. }) = &mut self.controls.drawer {
                *confirm = Some(HideConfirmation {
                    ids: ids.to_owned(),
                    skip_future: false,
                });
                self.focus_panel(window, cx);
                cx.notify();
            }
        } else {
            self.set_visible(ids, false, cx);
        }
    }

    fn hiding_clears_conditions(&self, ids: &[ColumnId]) -> bool {
        ids.iter().any(|&id| {
            uses_column(self.data.query(), id) || uses_column(self.requested_query(), id)
        })
    }

    fn set_visible(&mut self, ids: &[ColumnId], visible: bool, cx: &mut Context<Self>) {
        if visible {
            self.controls.pending_hides.retain(|id| !ids.contains(id));
        } else if self.hiding_clears_conditions(ids) {
            self.controls.pending_hides.extend(ids);
            self.edit_query(cx, |query| {
                query.filters.retain(|f| !ids.contains(&f.column));
                query.sorts.retain(|s| !ids.contains(&s.column));
            });
            return;
        }
        self.change_columns(
            false,
            |columns| columns.set_visible(ids.iter().copied().collect(), visible),
            cx,
        );
        cx.notify();
    }

    pub(in crate::table) fn apply_pending_hides(&mut self, cx: &mut Context<Self>) -> bool {
        let mut ids = std::mem::take(&mut self.controls.pending_hides);
        if ids.is_empty() {
            return false;
        }
        // A subsequent edit can restore a condition before the cleanup query finishes.
        ids.retain(|&id| !uses_column(self.data.query(), id));
        self.update_columns(false, |columns| columns.set_visible(ids, false), cx)
    }

    fn move_column(&mut self, id: ColumnId, by: isize, cx: &mut Context<Self>) {
        let Some(index) = self.data.columns.position(id) else {
            return;
        };
        let index = index.index();
        let Some(target) = index
            .checked_add_signed(by)
            .filter(|i| *i < self.data.columns.len())
        else {
            return;
        };
        self.change_columns(true, |columns| columns.move_to(index, target), cx);
        cx.notify();
    }

    fn change_columns(
        &mut self,
        reordered: bool,
        update: impl FnOnce(&mut TableColumns) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.update_columns(reordered, update, cx) {
            return false;
        }
        self.grid_changed(cx);
        true
    }

    fn update_columns(
        &mut self,
        reordered: bool,
        update: impl FnOnce(&mut TableColumns) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.selection.map(|s| {
            (
                s,
                self.data.columns[s.anchor.column].id,
                self.data.columns[s.head.column].id,
            )
        });
        if !update(&mut self.data.columns) {
            return false;
        }
        let columns = &self.data.columns;
        if self
            .filter_panel_column()
            .is_some_and(|id| columns.position(id).is_none())
        {
            self.controls.drawer = None;
        }
        let selection = selected
            .filter(|_| !columns.is_empty())
            .map(|(s, anchor, head)| {
                let relocate = |cell: Cell, id| Cell {
                    column: columns.position(id).map_or(0, |column| column.index()),
                    ..cell
                };
                Selection {
                    anchor: relocate(s.anchor, anchor),
                    head: relocate(s.head, head),
                }
            });
        self.set_selection(selection, cx);
        self.sync_find_columns(reordered, cx);
        true
    }
}
