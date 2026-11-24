use std::collections::HashSet;

use gewu_core::{ColumnId, ColumnSort};
use gpui_kit::{
    component::menu::{PopupMenu, PopupMenuItem},
    *,
};

use super::{ColumnDrag, Drawer, Order, TableView, button};
use crate::panels::choice::{Choice, ChoiceField, column_labels};

impl TableView {
    pub(in crate::table) fn open_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_panel(window, cx);
        self.controls.drawer = Some(Drawer::Sort { summary: false });
        self.controls.menu = None;
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

    fn sort_choice(
        &self,
        priority: Option<usize>,
        options: Vec<Choice<ColumnId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let replacing = priority
            .and_then(|i| self.requested_query().sorts.get(i))
            .map(|rule| rule.column);

        let pick = cx.listener(move |t, selected: &ColumnId, _, cx| {
            let selected = *selected;
            t.change_sorts(
                |rules| {
                    if rules.iter().any(|r| r.column == selected) {
                        return;
                    }
                    if let Some(index) = priority {
                        if let Some(rule) = rules.get_mut(index) {
                            rule.column = selected;
                        }
                    } else {
                        rules.push(ColumnSort {
                            column: selected,
                            descending: false,
                        });
                    }
                },
                cx,
            );
        });
        ChoiceField {
            id: priority.map_or_else(
                || ElementId::from("add-sort"),
                |index| ("sort-column", index).into(),
            ),
            label: if replacing.is_some() {
                "Sort column"
            } else {
                "Add sort column…"
            },
            options,
            selected: replacing,
            searchable: true,
        }
        .element(window, cx, pick)
    }

    fn sort_direction(
        &self,
        id: ColumnId,
        descending: bool,
        cx: &mut Context<Self>,
    ) -> base::RadioGroup {
        let raw = id.into_raw() as usize;
        let mut group = base::RadioGroup::new(("sort-direction", raw))
            .axis(Axis::Horizontal)
            .aria_label("Sort direction")
            .flex()
            .flex_none()
            .items_center()
            .w(px(76.))
            .p(px(2.))
            .rounded_md()
            .bg(base::Theme::global(cx).tokens.colors.muted);
        for (value, key, label, short) in [
            (false, "ascending", "Ascending", "Asc"),
            (true, "descending", "Descending", "Desc"),
        ] {
            let selected = descending == value;
            let choose = cx.listener(move |t, _, _, cx| t.set_direction(id, value, cx));
            let radio = base::Radio::new((key, raw))
                .checked(selected)
                .accessibility_label(label)
                .on_change(move |_, event, window, cx| choose(event, window, cx));
            group = group.child(crate::panels::segment(radio, short, selected, cx));
        }
        group
    }

    pub(super) fn sort_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = gpui_kit::base::Theme::global(cx).tokens.colors;
        let mut panel = self.drawer("Sort", window, cx);
        let sorts = &self.requested_query().sorts;
        let used: HashSet<_> = sorts.iter().map(|s| s.column).collect();
        let columns: Vec<_> = column_labels(self.data.columns.iter(), None)
            .map(|(value, label)| Choice { value, label })
            .collect();
        let options = |current: Option<ColumnId>| {
            columns
                .iter()
                .filter(|c| Some(c.value) == current || !used.contains(&c.value))
                .cloned()
                .collect()
        };
        let mut rows = div()
            .id("sort-list")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for (index, sort) in sorts.iter().enumerate() {
            let id = sort.column;
            let Some(col) = self.data.columns.iter().find(|c| c.id == id) else {
                continue;
            };
            let raw = id.into_raw() as usize;
            let descending = sort.descending;
            let payload = ColumnDrag {
                document: cx.entity_id(),
                order: Order::Sorting,
                index,
                id,
                name: col.name.clone().into(),
            };
            let row = div()
                .id(("sort-rule", raw))
                .test_support()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |t, event: &MouseDownEvent, window, cx| {
                        let Some(at) = t
                            .requested_query()
                            .sorts
                            .iter()
                            .position(|s| s.column == id)
                        else {
                            return;
                        };
                        let len = t.requested_query().sorts.len();
                        let weak = cx.weak_entity();
                        let return_focus = window
                            .focused(cx)
                            .or_else(|| t.controls.panel_focus.clone())
                            .unwrap_or_else(|| t.focus.clone());
                        let popup = PopupMenu::build(window, cx, |menu, _, _| {
                            let mut menu = menu.action_context(return_focus);
                            for (label, step, enabled) in
                                [("Move Up", -1, at > 0), ("Move Down", 1, at + 1 < len)]
                            {
                                let table = weak.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label).disabled(!enabled).on_click(
                                        move |_, _, cx| {
                                            let _ =
                                                table.update(cx, |t, cx| t.move_sort(id, step, cx));
                                        },
                                    ),
                                );
                            }
                            menu
                        });
                        t.present_menu(popup, event.position, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_drag(payload.clone(), |drag, _, _, cx| cx.new(|_| drag.clone()))
                .drag_over::<ColumnDrag>(move |style, drag, _, _| {
                    payload.insertion_indicator(style, drag, colors.primary)
                })
                .on_drop(cx.listener(move |t, drag: &ColumnDrag, _, cx| {
                    if drag.document != cx.entity_id() || drag.order != Order::Sorting {
                        return;
                    }
                    t.change_sorts(
                        |rules| {
                            if let (Some(from), Some(to)) = (
                                rules.iter().position(|r| r.column == drag.id),
                                rules.iter().position(|r| r.column == id),
                            ) {
                                let rule = rules.remove(from);
                                rules.insert(to, rule);
                            }
                        },
                        cx,
                    );
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(format!("⠿ {}", index + 1)),
                )
                .child(div().flex_1().min_w_0().child(self.sort_choice(
                    Some(index),
                    options(Some(id)),
                    window,
                    cx,
                )))
                .child(self.sort_direction(id, descending, cx))
                .child(
                    super::icon_button(("earlier-sort", raw), "Move rule earlier", "↑", cx)
                        .disabled(index == 0)
                        .on_click(cx.listener(move |t, _, _, cx| t.move_sort(id, -1, cx))),
                )
                .child(
                    super::icon_button(("later-sort", raw), "Move rule later", "↓", cx)
                        .disabled(index + 1 == sorts.len())
                        .on_click(cx.listener(move |t, _, _, cx| t.move_sort(id, 1, cx))),
                )
                .child(
                    super::icon_button(("remove-sort", raw), "Remove rule", "×", cx).on_click(
                        cx.listener(move |t, _, _, cx| {
                            t.change_sorts(|s| s.retain(|r| r.column != id), cx)
                        }),
                    ),
                );
            rows = rows.child(row);
        }
        panel = panel.child(rows);
        panel = panel.child(self.sort_choice(None, options(None), window, cx));
        let mut footer = super::footer();
        if !sorts.is_empty() {
            footer = footer.child(
                button("clear-sorts", "Clear sorting", false, cx)
                    .on_click(cx.listener(|t, _, _, cx| t.change_sorts(Vec::clear, cx))),
            );
        }
        panel
            .child(footer.child(self.close_button("Done", cx)))
            .into_any_element()
    }
}
