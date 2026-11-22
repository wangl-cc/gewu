use gewu_core::ColumnId;
use gpui_kit::{
    base::input::{Input, InputEvent, InputState},
    *,
};

use super::{Cell, ColumnDrag, Drawer, Order, Preferences, Selection, TableView, button};
use crate::columns::TableColumns;

impl TableView {
    pub(in crate::table) fn open_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find column"));
        let events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.controls.menu = None;
        self.controls.drawer = Some(Drawer::Columns {
            search,
            _events: events,
            confirm: None,
        });
        cx.notify();
    }

    pub(super) fn columns_panel(
        &self,
        search: &Entity<InputState>,
        confirm: Option<&[ColumnId]>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = self.drawer("Visible columns", cx);
        if let Some(ids) = confirm {
            return panel
                .child(format!(
                    "Hide {} columns and remove their filters and sorts?",
                    ids.len()
                ))
                .child(
                    button("cancel-hide", "Cancel", false, cx).on_click(cx.listener(
                        |t, _, _, cx| {
                            if let Some(Drawer::Columns { confirm, .. }) = &mut t.controls.drawer {
                                *confirm = None;
                            }
                            cx.notify();
                        },
                    )),
                )
                .child(
                    button("confirm-hide", "Hide", false, cx)
                        .on_click(cx.listener(|t, _, _, cx| t.confirm_hide(false, cx))),
                )
                .child(
                    button("always-hide", "Hide and don’t ask again", false, cx)
                        .on_click(cx.listener(|t, _, _, cx| t.confirm_hide(true, cx))),
                )
                .into_any_element();
        }
        let query = search.read(cx).value().to_lowercase();
        let mut all = Vec::new();
        let mut matches = Vec::new();
        for (position, column) in self.data.columns.visible_then_hidden() {
            all.push(column.id);
            if query.is_empty() || column.name.to_lowercase().contains(&query) {
                matches.push((position, column));
            }
        }
        let matching: Vec<_> = matches.iter().map(|(_, column)| column.id).collect();
        let to_show = matching.clone();
        panel =
            panel
                .child(
                    div()
                        .w_full()
                        .h(px(28.))
                        .flex_none()
                        .child(Input::new(search)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            button("show-matching", "Show matching", false, cx).on_click(
                                cx.listener(move |t, _, _, cx| t.set_visible(&to_show, true, cx)),
                            ),
                        )
                        .child(
                            button("hide-matching", "Hide matching", false, cx).on_click(
                                cx.listener(move |t, _, _, cx| t.request_hide(&matching, cx)),
                            ),
                        )
                        .child(button("show-all-columns", "Show all", false, cx).on_click(
                            cx.listener(move |t, _, _, cx| t.set_visible(&all, true, cx)),
                        )),
                );
        for (position, column) in matches {
            let id = column.id;
            let raw = id.into_raw() as usize;
            let mut row = div()
                .id(("column-rule", raw))
                .flex()
                .items_center()
                .gap_1()
                .child(crate::panels::toggle(
                    ("toggle-column", raw),
                    format!("{} · {}", column.name, column.display_type),
                    position.is_some(),
                    cx,
                    cx.listener(move |t, _, _, cx| {
                        if t.data.columns.position(id).is_some() {
                            t.request_hide(&[id], cx);
                        } else {
                            t.set_visible(&[id], true, cx);
                        }
                    }),
                ));
            if let Some(position) = position {
                let index = position.index();
                let payload = ColumnDrag {
                    document: cx.entity_id(),
                    order: Order::Columns,
                    id,
                    name: column.name.clone().into(),
                };
                row = row
                    .on_drag(payload, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .on_drop(cx.listener(move |t, drag: &ColumnDrag, _, cx| {
                        if drag.document != cx.entity_id() || drag.order != Order::Columns {
                            return;
                        }
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
                if index > 0 {
                    row = row.child(
                        button(("column-earlier", raw), "Earlier", false, cx)
                            .on_click(cx.listener(move |t, _, _, cx| t.move_column(id, -1, cx))),
                    );
                }
                if index + 1 < self.data.columns.len() {
                    row = row.child(
                        button(("column-later", raw), "Later", false, cx)
                            .on_click(cx.listener(move |t, _, _, cx| t.move_column(id, 1, cx))),
                    );
                }
            }
            panel = panel.child(row);
        }
        panel.into_any_element()
    }

    fn confirm_hide(&mut self, skip: bool, cx: &mut Context<Self>) {
        let Some(Drawer::Columns { confirm, .. }) = &mut self.controls.drawer else {
            return;
        };
        let Some(ids) = confirm.take() else { return };
        if skip {
            Preferences::update(cx, |p| p.skip_hide_warning = true);
        }
        self.set_visible(&ids, false, cx);
        cx.notify();
    }

    pub(super) fn request_hide(&mut self, ids: &[ColumnId], cx: &mut Context<Self>) {
        let conditioned = [self.requested_query(), self.data.query().as_ref()]
            .into_iter()
            .any(|query| {
                query.sorts.iter().any(|s| ids.contains(&s.column))
                    || query.filters.iter().any(|f| ids.contains(&f.column))
            });
        if conditioned && !Preferences::get(cx).skip_hide_warning {
            if let Some(Drawer::Columns { confirm, .. }) = &mut self.controls.drawer {
                *confirm = Some(ids.to_owned());
                cx.notify();
            }
        } else {
            self.set_visible(ids, false, cx);
        }
    }

    fn set_visible(&mut self, ids: &[ColumnId], visible: bool, cx: &mut Context<Self>) {
        if !self.change_columns(false, |columns| columns.set_visible(ids, visible), cx) {
            return;
        }
        if !visible
            && (self
                .requested_query()
                .filters
                .iter()
                .any(|f| ids.contains(&f.column))
                || self
                    .requested_query()
                    .sorts
                    .iter()
                    .any(|s| ids.contains(&s.column)))
        {
            self.edit_query(cx, |query| {
                query.filters.retain(|f| !ids.contains(&f.column));
                query.sorts.retain(|s| !ids.contains(&s.column));
            });
        }
        cx.notify();
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
        let selected = self.selection.and_then(|s| {
            Some((
                s,
                self.data.columns.get(s.anchor.column)?.id,
                self.data.columns.get(s.head.column)?.id,
            ))
        });
        if !update(&mut self.data.columns) {
            return false;
        }
        self.selection = selected.and_then(|(s, anchor, head)| {
            let relocate = |cell: Cell, id| {
                let column = self
                    .data
                    .columns
                    .position(id)
                    .map(|column| column.index())
                    .or_else(|| (!self.data.columns.is_empty()).then_some(0))?;
                Some(Cell { column, ..cell })
            };
            Some(Selection {
                anchor: relocate(s.anchor, anchor)?,
                head: relocate(s.head, head)?,
            })
        });
        self.visible_columns_changed(reordered, cx);
        self.sync_panels(cx);
        true
    }
}
