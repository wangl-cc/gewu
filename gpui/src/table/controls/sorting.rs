use gewu_core::{ColumnId, ColumnSort};
use gpui_kit::*;

use super::{ColumnDrag, Drawer, Order, TableView, button};

impl TableView {
    pub(in crate::table) fn open_sort(&mut self, cx: &mut Context<Self>) {
        self.controls.drawer = Some(Drawer::Sort(None));
        self.controls.menu = None;
        self.controls.replace_sort = None;
        self.controls.sort_menu = None;
        cx.notify();
    }

    pub(super) fn change_sorts(
        &mut self,
        edit: impl FnOnce(&mut Vec<ColumnSort>),
        cx: &mut Context<Self>,
    ) {
        self.edit_query(cx, |query| edit(&mut query.sorts));
    }

    pub(super) fn set_direction(&mut self, id: ColumnId, descending: bool, cx: &mut Context<Self>) {
        self.change_sorts(
            |sorts| {
                if let Some(s) = sorts.iter_mut().find(|s| s.column == id) {
                    s.descending = descending;
                } else {
                    sorts.push(ColumnSort {
                        column: id,
                        descending,
                    });
                }
            },
            cx,
        );
    }

    fn move_sort(&mut self, id: ColumnId, by: isize, cx: &mut Context<Self>) {
        self.change_sorts(
            |sorts| {
                if let Some(index) = sorts.iter().position(|s| s.column == id)
                    && let Some(target) = index.checked_add_signed(by).filter(|i| *i < sorts.len())
                {
                    sorts.swap(index, target);
                }
            },
            cx,
        );
    }

    pub(super) fn sort_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = self.drawer("Sort", cx);
        for (index, sort) in self.requested_query().sorts.iter().enumerate() {
            let id = sort.column;
            let Some(col) = self.data.columns.iter().find(|c| c.id == id) else {
                continue;
            };
            let raw = id.into_raw() as usize;
            let descending = sort.descending;
            let payload = ColumnDrag {
                document: cx.entity_id(),
                order: Order::Sorting,
                id,
                name: col.name.clone().into(),
            };
            let mut row = div()
                .id(("sort-rule", raw))
                .flex()
                .flex_wrap()
                .gap_1()
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |t, event: &MouseDownEvent, _, cx| {
                        t.controls.sort_menu = Some((id, event.position));
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .on_drag(payload, |drag, _, _, cx| cx.new(|_| drag.clone()))
                .on_drop(cx.listener(move |t, drag: &ColumnDrag, _, cx| {
                    if drag.document == cx.entity_id() && drag.order == Order::Sorting {
                        t.change_sorts(
                            |rules| {
                                let from = rules.iter().position(|r| r.column == drag.id);
                                let to = rules.iter().position(|r| r.column == id);
                                if let (Some(from), Some(to)) = (from, to) {
                                    let rule = rules.remove(from);
                                    rules.insert(to, rule);
                                }
                            },
                            cx,
                        );
                    }
                }))
                .child(
                    button(
                        ("sort-column", raw),
                        format!("{}. {}", index + 1, col.name),
                        self.controls.replace_sort == Some(id),
                        cx,
                    )
                    .aria_expanded(self.controls.replace_sort == Some(id))
                    .on_click(cx.listener(move |t, _, _, cx| {
                        t.controls.replace_sort = if t.controls.replace_sort == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        ("direction", raw),
                        if descending {
                            "Descending"
                        } else {
                            "Ascending"
                        },
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |t, _, _, cx| t.set_direction(id, !descending, cx))),
                )
                .child(
                    button(("remove-sort", raw), "Remove", false, cx).on_click(cx.listener(
                        move |t, _, _, cx| t.change_sorts(|s| s.retain(|r| r.column != id), cx),
                    )),
                );
            if index > 0 {
                row = row.child(
                    button(("earlier-sort", raw), "Earlier", false, cx)
                        .on_click(cx.listener(move |t, _, _, cx| t.move_sort(id, -1, cx))),
                );
            }
            if index + 1 < self.requested_query().sorts.len() {
                row = row.child(
                    button(("later-sort", raw), "Later", false, cx)
                        .on_click(cx.listener(move |t, _, _, cx| t.move_sort(id, 1, cx))),
                );
            }
            panel = panel.child(row);
            if self.controls.replace_sort == Some(id) {
                for column in self.data.columns.iter().filter(|c| {
                    !self
                        .requested_query()
                        .sorts
                        .iter()
                        .any(|s| s.column == c.id)
                }) {
                    let new_id = column.id;
                    panel = panel.child(
                        button(
                            ("replace-sort", new_id.into_raw() as usize),
                            column.name.clone(),
                            false,
                            cx,
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.controls.replace_sort = None;
                            t.change_sorts(
                                |rules| {
                                    if !rules.iter().any(|r| r.column == new_id)
                                        && let Some(rule) =
                                            rules.iter_mut().find(|r| r.column == id)
                                    {
                                        rule.column = new_id;
                                    }
                                },
                                cx,
                            );
                        })),
                    );
                }
            }
        }
        if !self.requested_query().sorts.is_empty() {
            panel = panel.child(
                button("clear-sorts", "Clear sorting", false, cx)
                    .on_click(cx.listener(|t, _, _, cx| t.change_sorts(Vec::clear, cx))),
            );
        }
        for col in self.data.columns.iter().filter(|c| {
            !self
                .requested_query()
                .sorts
                .iter()
                .any(|s| s.column == c.id)
        }) {
            let id = col.id;
            panel = panel.child(
                button(
                    ("add-sort", id.into_raw() as usize),
                    format!("Add {}", col.name),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |t, _, _, cx| t.set_direction(id, false, cx))),
            );
        }
        if let Some((id, position)) = self.controls.sort_menu
            && let Some(at) = self
                .requested_query()
                .sorts
                .iter()
                .position(|s| s.column == id)
        {
            let colors = gpui_kit::base::Theme::global(cx).tokens.colors;
            let mut menu = div()
                .id("sort-rule-menu")
                .occlude()
                .flex()
                .flex_col()
                .p_1()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface)
                .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                    t.controls.sort_menu = None;
                    cx.notify();
                }));
            for (label, step, enabled) in [
                ("Move Up", -1, at > 0),
                ("Move Down", 1, at + 1 < self.requested_query().sorts.len()),
            ] {
                menu = menu.child(button(label, label, false, cx).disabled(!enabled).on_click(
                    cx.listener(move |t, _, _, cx| {
                        t.controls.sort_menu = None;
                        t.move_sort(id, step, cx);
                    }),
                ));
            }
            panel = panel.child(
                deferred(anchored().position(position).snap_to_window().child(menu))
                    .with_priority(12),
            );
        }
        panel.into_any_element()
    }
}
