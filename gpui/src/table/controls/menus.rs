use gewu_core::ColumnId;
use gpui_kit::{
    base::{self, TestSupportExt},
    component::{
        kbd::Kbd,
        menu::{PopupMenu, PopupMenuItem},
    },
    prelude::FluentBuilder,
    *,
};

use super::{Cell, Drawer, Selection, TableView, button};
use crate::table::HEADER_HEIGHT;

#[derive(Clone, Copy)]
struct MenuTarget {
    column: ColumnId,
    cell: Option<Cell>,
}

pub(super) struct OpenMenu {
    position: Point<Pixels>,
    popup: Entity<PopupMenu>,
    return_focus: FocusHandle,
    _dismiss: Subscription,
}

impl OpenMenu {
    pub(super) fn element(&self, window: &Window, cx: &App) -> AnyElement {
        deferred(
            anchored().position(self.position).snap_to_window().child(
                crate::panels::elevated(div(), window, cx)
                    .overflow_hidden()
                    .child(self.popup.clone()),
            ),
        )
        .with_priority(base::POPUP_PRIORITY + 1)
        .into_any_element()
    }
}

/// PopupMenu owns keyboard selection, dismissal and click dispatch. AccessKit names a
/// MenuItem from its Label child's value.
pub(super) fn menu_item(
    label: &'static str,
    enabled: bool,
    shortcut: Option<Kbd>,
) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        let palette = crate::theme::GewuPalette::get(cx);
        // ElementItem inserts 0.5rem of horizontal padding around custom content.
        // Extend the press target across that padding, keeping the label's normal inset.
        div()
            .id("menu-content")
            .role(Role::Label)
            .aria_value(label)
            .test_support()
            .flex_1()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .mx_neg_2()
            .px_2()
            .h(px(26.))
            .rounded(px(6.))
            // GPUI registers the row's hitbox for its cursor; active styling alone
            // does not register one. Native menus keep the arrow over their items.
            .cursor_default()
            .when(enabled, |row| {
                row.active(move |s| s.bg(palette.menu_pressed()))
            })
            .when(!enabled, |row| row.opacity(crate::theme::DISABLED_OPACITY))
            .child(label)
            .children(
                shortcut
                    .clone()
                    .map(|key| key.p_0().flex_nowrap().border_0().bg(Background::default())),
            )
    })
    .disabled(!enabled)
}

#[derive(Clone, Copy)]
enum Command {
    CopyCell,
    CopyRows,
    Detail,
    Filter,
    ClearFilter,
    Analyze,
    Ascending,
    Descending,
    ClearSort,
    ClearSorting,
    Columns,
    Hide,
}

impl TableView {
    pub(in crate::table) fn context_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        let local = event.position - self.layout.bounds.origin;
        if self.scrollbar_at(local, self.show_scrollbars(cx)).is_some() {
            return;
        }
        let x = local.x.as_f32();
        let y = local.y.as_f32();
        let Some(column) = self.layout.column_at(x) else {
            return;
        };
        let cell = self.layout.cell_at(x, y);
        if cell.is_none() && !(0.0..HEADER_HEIGHT).contains(&y) {
            return;
        }
        if let Some(cell) = cell
            && !self.selection.is_some_and(|s| s.rows().contains(&cell.row))
        {
            self.set_selection(Some(Selection::single(cell)), cx);
        }
        let target = MenuTarget {
            column: self.data.columns[column].id,
            cell,
        };
        let focus = self.focus.clone();
        let has_sort = self
            .requested_query()
            .sorts
            .iter()
            .any(|s| s.column == target.column);
        let sorted = !self.requested_query().sorts.is_empty();
        let filtered = self
            .requested_query()
            .filters
            .iter()
            .any(|f| f.column == target.column);
        let table = cx.weak_entity();
        let popup = PopupMenu::build(window, cx, |menu, window, _| {
            use Command::*;
            let item = |label: &'static str, command| {
                let table = table.clone();
                let enabled = match command {
                    ClearFilter => filtered,
                    ClearSort => has_sort,
                    ClearSorting => sorted,
                    _ => true,
                };
                let action: Option<Box<dyn Action>> = match command {
                    CopyRows => Some(Box::new(base::input::Copy)),
                    Filter => Some(Box::new(crate::actions::EditFilter)),
                    Analyze => Some(Box::new(crate::actions::AnalyzeSelectedColumn)),
                    _ => None,
                };
                let shortcut = action.as_deref().and_then(|action| {
                    Kbd::binding_for_action_in(action, &focus, window)
                        .or_else(|| Kbd::global_binding_for_action(action, window))
                });
                menu_item(label, enabled, shortcut)
                    .when_some(action, |item, action| item.action(action))
                    .on_click(move |_, w, cx| {
                        let _ = table.update(cx, |t, cx| t.menu_command(command, target, w, cx));
                    })
            };
            let menu = menu.min_w(px(194.)).action_context(focus.clone());
            let menu = if target.cell.is_some() {
                menu.item(item("Copy Cell", CopyCell))
                    .item(item("Copy Selected Rows", CopyRows))
                    .item(item("View Cell…", Detail))
            } else {
                menu.item(item("Sort Ascending", Ascending))
                    .item(item("Sort Descending", Descending))
                    .item(item("Clear Column Sort", ClearSort))
                    .item(item("Clear Sorting", ClearSorting))
            };
            let menu = menu
                .separator()
                .item(item("Filter Column…", Filter))
                .item(item("Clear Column Filter", ClearFilter))
                .item(item("Analyze Column", Analyze));
            if target.cell.is_none() {
                menu.separator()
                    .item(item("Columns…", Columns))
                    .item(item("Hide Column", Hide))
            } else {
                menu
            }
        });
        let anchor = if target.cell.is_some() {
            event.position + point(px(4.), px(12.))
        } else {
            point(
                event.position.x,
                self.layout.bounds.top() + px(HEADER_HEIGHT + 2.),
            )
        };
        self.present_menu(popup, anchor, window, cx);
        cx.stop_propagation();
    }

    pub(in crate::table) fn window_activation_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active()
            && let Some(menu) = self.controls.menu.take()
        {
            window.focus(&menu.return_focus, cx);
        }
        cx.notify();
    }

    pub(super) fn present_menu(
        &mut self,
        popup: Entity<PopupMenu>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismiss = cx.subscribe(&popup, |t, _, _: &DismissEvent, cx| {
            t.controls.menu = None;
            cx.notify();
        });
        let return_focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        window.focus(&popup.focus_handle(cx), cx);
        self.controls.menu = Some(OpenMenu {
            position,
            popup,
            return_focus,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    fn menu_command(
        &mut self,
        command: Command,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use Command::*;
        let id = target.column;
        match command {
            CopyCell => {
                if let Some(cell) = target.cell
                    && let Some(value) = self.data.cell(cell.column, cell.row)
                {
                    self.copy_cell_text(value.unwrap_or_default().to_string(), cx);
                }
            }
            CopyRows => self.copy(window, cx),
            Detail => {
                if let Some(cell) = target.cell {
                    self.show_cell(cell, window, cx);
                }
            }
            Filter => self.filter_column(id, window, cx),
            ClearFilter => self.set_filter(id, None, cx),
            Analyze => self.analyze_column(id, cx),
            Ascending => self.set_direction(id, false, cx),
            Descending => self.set_direction(id, true, cx),
            ClearSort => self.change_sorts(|s| s.retain(|r| r.column != id), cx),
            ClearSorting => self.change_sorts(Vec::clear, cx),
            Columns => self.open_columns(window, cx),
            Hide => {
                self.open_columns(window, cx);
                self.request_hide(&[id], window, cx);
            }
        }
    }

    pub(in crate::table) fn show_cell(
        &mut self,
        cell: Cell,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        let Some(value) = self.data.cell(cell.column, cell.row) else {
            return;
        };
        let anchor = self.layout.bounds.origin
            + point(
                px(self.column_left(cell.column).unwrap_or(self.layout.gutter)),
                px(HEADER_HEIGHT + self.layout.vertical.row_top(cell.row + 1) as f32 + 4.),
            );
        self.set_selection(Some(Selection::single(cell)), cx);
        self.focus_panel(window, cx);
        self.controls.drawer = Some(Drawer::Cell { anchor, value });
        cx.notify();
    }

    pub(super) fn cell_panel(
        &self,
        value: Option<&SharedString>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = value.cloned().unwrap_or_default();
        let colors = crate::theme::GewuPalette::get(cx);
        let viewport = window.viewport_size();
        crate::panels::elevated(div().id("cell-preview"), window, cx)
            .test_support()
            .max_w(px(420.).min(viewport.width - px(12.)))
            .max_h(px(280.).min(viewport.height - px(12.)))
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(
                div()
                    .id("cell-detail-text")
                    .min_h_0()
                    .overflow_y_scroll()
                    .text_size(px(crate::preferences::Preferences::get(cx).font_size))
                    .font_features(crate::theme::tabular_digits())
                    .when(value.is_none(), |s| {
                        s.italic().text_color(colors.muted_foreground)
                    })
                    .child(value.cloned().unwrap_or_else(|| "null".into())),
            )
            .child(
                super::footer().child(
                    button("copy-cell", "Copy", false, cx)
                        .accessibility_label("Copy cell value")
                        .on_click(cx.listener(move |table, _, _, cx| {
                            table.copy_cell_text(copy.to_string(), cx)
                        })),
                ),
            )
            .into_any_element()
    }
}
