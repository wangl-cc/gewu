use gewu_core::ColumnId;
use gpui_kit::{
    base,
    component::menu::{PopupMenu, PopupMenuItem},
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
    _dismiss: Subscription,
}

impl OpenMenu {
    pub(super) fn element(&self) -> AnyElement {
        deferred(
            anchored()
                .position(self.position)
                .snap_to_window()
                .child(self.popup.clone()),
        )
        .with_priority(base::POPUP_PRIORITY + 1)
        .into_any_element()
    }
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
        let local = event.position - self.layout.bounds.origin;
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
            self.selection = Some(Selection::single(cell));
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
        let item = move |label: &'static str, command| {
            let table = table.clone();
            let item = PopupMenuItem::new(label).on_click(move |_, w, cx| {
                let _ = table.update(cx, |t, cx| t.menu_command(command, target, w, cx));
            });
            match command {
                Command::CopyRows => item.action(Box::new(base::input::Copy)),
                Command::Filter => item.action(Box::new(crate::actions::EditFilter)),
                Command::Analyze => item.action(Box::new(crate::actions::AnalyzeSelectedColumn)),
                _ => item,
            }
        };
        let popup = PopupMenu::build(window, cx, |menu, _, _| {
            use Command::*;
            let menu = menu.min_w(px(194.)).action_context(focus);
            let menu = if target.cell.is_some() {
                menu.item(item("Copy Cell", CopyCell))
                    .item(item("Copy Selected Rows", CopyRows))
                    .separator()
                    .item(item("View Cell…", Detail))
            } else {
                menu.item(item("Sort Ascending", Ascending))
                    .item(item("Sort Descending", Descending))
                    .item(item("Clear Column Sort", ClearSort).disabled(!has_sort))
                    .item(item("Clear Sorting", ClearSorting).disabled(!sorted))
            };
            let menu = menu
                .separator()
                .item(item("Filter Column…", Filter))
                .item(item("Clear Column Filter", ClearFilter).disabled(!filtered))
                .item(item("Analyze Column", Analyze));
            if target.cell.is_none() {
                menu.separator()
                    .item(item("Columns…", Columns))
                    .item(item("Hide Column", Hide))
            } else {
                menu
            }
        });
        self.present_menu(popup, event.position, window, cx);
        cx.stop_propagation();
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
        window.focus(&popup.focus_handle(cx), cx);
        self.controls.menu = Some(OpenMenu {
            position,
            popup,
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
        let Some(value) = self.data.cell(cell.column, cell.row) else {
            return;
        };
        let anchor = self.layout.bounds.origin
            + point(
                px(self.column_left(cell.column).unwrap_or(self.layout.gutter)),
                px(HEADER_HEIGHT + self.layout.vertical.row_top(cell.row + 1) as f32),
            );
        self.focus_panel(window, cx);
        self.controls.drawer = Some(Drawer::Cell {
            anchor,
            name: self.data.columns[cell.column].name.clone(),
            value,
        });
        cx.notify();
    }

    pub(super) fn cell_panel(
        &self,
        name: &str,
        value: Option<&SharedString>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = value.cloned().unwrap_or_default();
        self.drawer("Cell details", window, cx)
            .overflow_hidden()
            .child(div().flex_none().child(name.to_owned()))
            .child(
                div()
                    .id("cell-detail-text")
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        value
                            .cloned()
                            .unwrap_or_else(|| SharedString::new_static("null")),
                    ),
            )
            .child(
                super::footer().child(self.close_button("Close", cx)).child(
                    button("copy-cell", "Copy Cell", false, cx)
                        .flex_none()
                        .on_click(cx.listener(move |table, _, _, cx| {
                            table.copy_cell_text(copy.to_string(), cx)
                        })),
                ),
            )
            .into_any_element()
    }
}
