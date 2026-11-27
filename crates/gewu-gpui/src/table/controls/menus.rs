//! The grid's context menus and the cell detail popover.
use gewu_core::ViewRow;
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
use crate::{
    actions::{
        AnalyzeColumn, AnalyzeSelectedColumn, ClearColumnFilter, ClearColumnSort, ClearSorting,
        CopyCell, EditFilter, FilterColumn, HideColumn, ManageColumns, SortColumn, ViewCell,
    },
    table::{HEADER_HEIGHT, grid::GridHit},
};

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
                crate::ui::elevated(div(), window, cx)
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
            .text_size(px(13.))
            // GPUI registers the row's hitbox for its cursor; active styling alone
            // does not register one. Native menus keep the arrow over their items.
            .cursor_default()
            .when(enabled, |row| {
                row.active(move |s| s.bg(palette.menu_pressed()))
            })
            .when(!enabled, |row| row.opacity(crate::theme::DISABLED_OPACITY))
            .child(label)
            .children(shortcut.clone().map(|key| {
                key.p_0()
                    .flex_nowrap()
                    .border_0()
                    .bg(Background::default())
                    .text_size(px(12.))
                    .text_color(palette.muted_foreground)
            }))
    })
    .disabled(!enabled)
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
        let (column, cell) = match self.grid.hit(local, &self.layout, self.show_scrollbars(cx)) {
            GridHit::Cell(cell) => (cell.column, Some(cell)),
            GridHit::Header(part) => (part.column(), None),
            GridHit::Scrollbar(..) | GridHit::Outside => return,
        };
        if let Some(cell) = cell
            && !self
                .grid
                .selection()
                .is_some_and(|s| s.rows().contains(&cell.row))
        {
            self.set_selection(Some(Selection::single(cell)), cx);
        }
        let column_id = self.data.columns[column].id;
        let target = cell.map(|cell| (ViewRow(cell.row), column_id));
        let focus = self.focus.clone();
        // Which commands apply to this column, decided once from the requested query.
        let query = self.requested_query();
        let has_sort = query.sorts.iter().any(|s| s.column == column_id);
        let sorted = !query.sorts.is_empty();
        let filtered = query.filters.iter().any(|f| f.column == column_id);
        let popup = PopupMenu::build(window, cx, |menu, window, _| {
            let item = |label: &'static str,
                        action: Box<dyn Action>,
                        enabled: bool,
                        shortcut: Option<&dyn Action>| {
                let shortcut = shortcut.and_then(|action| {
                    Kbd::binding_for_action_in(action, &focus, window)
                        .or_else(|| Kbd::global_binding_for_action(action, window))
                });
                menu_item(label, enabled, shortcut).action(action)
            };
            let column = column_id;
            let menu = menu.min_w(px(194.)).action_context(focus.clone());
            let menu = if let Some((row, column)) = target {
                menu.item(item(
                    "Copy Cell",
                    Box::new(CopyCell { row, column }),
                    true,
                    None,
                ))
                .item(item(
                    "Copy Selected Rows",
                    Box::new(base::input::Copy),
                    true,
                    Some(&base::input::Copy),
                ))
                .item(item(
                    "View Cell…",
                    Box::new(ViewCell { row, column }),
                    true,
                    None,
                ))
            } else {
                menu.item(item(
                    "Sort Ascending",
                    Box::new(SortColumn {
                        column,
                        descending: false,
                    }),
                    true,
                    None,
                ))
                .item(item(
                    "Sort Descending",
                    Box::new(SortColumn {
                        column,
                        descending: true,
                    }),
                    true,
                    None,
                ))
                .item(item(
                    "Clear Column Sort",
                    Box::new(ClearColumnSort { column }),
                    has_sort,
                    None,
                ))
                .item(item("Clear Sorting", Box::new(ClearSorting), sorted, None))
            };
            let menu = menu
                .separator()
                .item(item(
                    "Filter Column…",
                    Box::new(FilterColumn { column }),
                    true,
                    Some(&EditFilter),
                ))
                .item(item(
                    "Clear Column Filter",
                    Box::new(ClearColumnFilter { column }),
                    filtered,
                    None,
                ))
                .item(item(
                    "Analyze Column",
                    Box::new(AnalyzeColumn { column }),
                    true,
                    Some(&AnalyzeSelectedColumn),
                ));
            if target.is_none() {
                menu.separator()
                    .item(item("Columns…", Box::new(ManageColumns), true, None))
                    .item(item(
                        "Hide Column",
                        Box::new(HideColumn { column }),
                        true,
                        None,
                    ))
            } else {
                menu
            }
        });
        let anchor = if target.is_some() {
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

    /// Handlers for the context menu's commands, on the table's root.
    pub(in crate::table) fn menu_actions(&self, view: Div, cx: &mut Context<Self>) -> Div {
        view.on_action(
            cx.listener(|t, a: &SortColumn, _, cx| t.set_direction(a.column, a.descending, cx)),
        )
        .on_action(cx.listener(|t, a: &ClearColumnSort, _, cx| {
            let id = a.column;
            t.change_sorts(|s| s.retain(|r| r.column != id), cx)
        }))
        .on_action(cx.listener(|t, _: &ClearSorting, _, cx| t.change_sorts(Vec::clear, cx)))
        .on_action(cx.listener(|t, a: &FilterColumn, w, cx| t.filter_column(a.column, w, cx)))
        .on_action(cx.listener(|t, a: &ClearColumnFilter, _, cx| t.set_filter(a.column, None, cx)))
        .on_action(cx.listener(|t, a: &HideColumn, w, cx| {
            t.open_columns(w, cx);
            t.request_hide(&[a.column], w, cx);
        }))
        .on_action(cx.listener(|t, a: &CopyCell, _, cx| {
            if let Some(cell) = t.data.display_cell(a.row, a.column)
                && let Some(value) = t.data.cell(cell.column, cell.row)
            {
                t.copy_cell_text(value.unwrap_or_default().to_string(), cx);
            }
        }))
        .on_action(cx.listener(|t, a: &ViewCell, w, cx| {
            if let Some(cell) = t.data.display_cell(a.row, a.column) {
                t.show_cell(cell, w, cx);
            }
        }))
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
                px(self
                    .layout
                    .column_left(cell.column)
                    .unwrap_or(self.layout.gutter)),
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
        crate::ui::elevated(div().id("cell-preview"), window, cx)
            .test_support()
            .min_w(px(160.).min(viewport.width - px(12.)))
            .max_w(px(420.).min(viewport.width - px(12.)))
            .max_h(px(280.).min(viewport.height - px(12.)))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
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
