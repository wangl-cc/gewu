use gewu_core::ColumnId;
use gpui_kit::{base, *};

use super::{Cell, Drawer, Selection, TableView, button};
use crate::table::HEADER_HEIGHT;

#[derive(Clone, Copy)]
pub(super) struct MenuTarget {
    position: Point<Pixels>,
    column: ColumnId,
    cell: Option<Cell>,
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
        self.controls.menu = Some(MenuTarget {
            position: event.position,
            column: self.data.columns[column].id,
            cell,
        });
        window.focus(&self.focus, cx);
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn menu_view(&self, target: MenuTarget, cx: &mut Context<Self>) -> AnyElement {
        use Command::*;
        let colors = base::Theme::global(cx).tokens.colors;
        let mut menu = div()
            .id("grid-context-menu")
            .occlude()
            .flex()
            .flex_col()
            .p_1()
            .gap_1()
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .rounded_md()
            .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                t.controls.menu = None;
                cx.notify();
            }));
        let mut items = Vec::new();
        if target.cell.is_some() {
            items.extend([
                ("Copy Cell", CopyCell),
                ("Copy Selected Rows", CopyRows),
                ("View Cell…", Detail),
            ]);
        } else {
            items.extend([
                ("Sort Ascending", Ascending),
                ("Sort Descending", Descending),
            ]);
            if self
                .requested_query()
                .sorts
                .iter()
                .any(|s| s.column == target.column)
            {
                items.push(("Clear Column Sort", ClearSort));
            }
            if !self.requested_query().sorts.is_empty() {
                items.push(("Clear Sorting", ClearSorting));
            }
        }
        items.push(("Filter Column…", Filter));
        if self
            .requested_query()
            .filters
            .iter()
            .any(|f| f.column == target.column)
        {
            items.push(("Clear Column Filter", ClearFilter));
        }
        items.push(("Analyze Column", Analyze));
        if target.cell.is_none() {
            items.extend([("Columns…", Columns), ("Hide Column", Hide)]);
        }
        for (label, command) in items {
            menu = menu.child(button(label, label, false, cx).on_click(cx.listener(
                move |t, _, w, cx| {
                    t.controls.menu = None;
                    t.menu_command(command, target, w, cx);
                    cx.notify();
                },
            )));
        }
        deferred(
            anchored()
                .position(target.position)
                .snap_to_window()
                .child(menu),
        )
        .into_any_element()
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
                    self.show_cell(cell, cx);
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
                self.request_hide(&[id], cx);
            }
        }
    }

    pub(in crate::table) fn show_cell(&mut self, cell: Cell, cx: &mut Context<Self>) {
        let Some(value) = self.data.cell(cell.column, cell.row) else {
            return;
        };
        let anchor = self.layout.bounds.origin
            + point(
                px(self.column_left(cell.column).unwrap_or(self.layout.gutter)),
                px(HEADER_HEIGHT + self.layout.vertical.row_top(cell.row + 1) as f32),
            );
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = value.cloned().unwrap_or_default();
        self.drawer("Cell details", cx)
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
                button("copy-cell", "Copy Cell", false, cx)
                    .flex_none()
                    .on_click(cx.listener(move |table, _, _, cx| {
                        table.copy_cell_text(copy.to_string(), cx)
                    })),
            )
            .into_any_element()
    }
}
