//! The visible cells of the canvas share their geometry and data with the accessibility tree.
//! Keeping keyboard focus on the grid lets the same selection drive keyboard and AX navigation.

use gpui_kit::*;

use super::{Cell, HEADER_HEIGHT, Layout, Selection, TableView};

pub(super) fn grid(table: &TableView, window: &Window) -> Stateful<Div> {
    let grid = div()
        .id("data-table")
        .relative()
        .role(Role::Grid)
        .aria_label("Data table");
    if !window.is_a11y_active() {
        return grid;
    }

    let row_count = table.data.row_count();
    let mut grid = grid
        .aria_column_count(table.data.columns.len())
        .aria_description(format!(
            "{}, {}",
            crate::counted(row_count, "row"),
            crate::counted(table.data.columns.len() as u64, "column")
        ));
    // AccessKit indices are zero-based. The column-header row occupies index zero.
    if let Some(count) = usize::try_from(row_count)
        .ok()
        .and_then(|n| n.checked_add(1))
    {
        grid = grid.aria_row_count(count);
    }

    grid
}

pub(super) fn rows(table: &mut TableView, layout: &Layout, cx: &Context<TableView>) -> AnyElement {
    let mut headers = div()
        .id("column-headers")
        .role(Role::Row)
        .aria_row_index(0)
        .absolute()
        .top_0()
        .w_full()
        .h(px(HEADER_HEIGHT.min(layout.body_bottom()).max(0.)));
    for (index, left, width) in layout.clipped_columns() {
        let column = &table.data.columns[index];
        let id = column.id;
        let entity = cx.weak_entity();
        let mut header = div()
            .id(("column", id.into_raw()))
            .role(Role::ColumnHeader)
            .aria_column_index(index)
            .aria_label(table.data.columns.entry(index).label.clone())
            .absolute()
            .left(px(left))
            .w(px(width))
            .h_full()
            .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                _ = entity.update(cx, |table, cx| {
                    if table.data.columns.get(index).is_some_and(|c| c.id == id) {
                        window.focus(&table.focus, cx);
                        table.cycle_sort(index, false, cx);
                    }
                });
            });
        if let Some(sort) = table.data.query().sorts.iter().find(|s| s.column == id) {
            header = header.aria_description(if sort.descending {
                "Sorted descending"
            } else {
                "Sorted ascending"
            });
        }
        headers = headers.child(header);
    }
    let mut overlay = div().relative().size_full().child(headers);

    for row in layout.vertical.visible_rows() {
        let top = HEADER_HEIGHT + layout.vertical.row_top(row) as f32;
        let bottom = (top + table.prefs.row_height).min(layout.body_bottom());
        let top = top.max(HEADER_HEIGHT);
        if top >= bottom {
            continue;
        }
        let source = table.data.source_row(row);
        let mut row_node = div()
            .id(("source-row", source))
            .role(Role::Row)
            .aria_label(format!("Row {}", source + table.prefs.row_number_base))
            .absolute()
            .top(px(top))
            .h(px(bottom - top))
            .w_full();
        if let Some(index) = usize::try_from(row).ok().and_then(|n| n.checked_add(1)) {
            row_node = row_node.aria_row_index(index);
        }
        for (column, left, width) in layout.clipped_columns() {
            let cell = Cell { row, column };
            let Some(value) = table.data.cell(column, row) else {
                continue;
            };
            let text = match value {
                None => SharedString::new_static("Missing value"),
                Some(text) if text.is_empty() => SharedString::new_static("Empty string"),
                Some(text) => text,
            };
            let id = table.data.columns[column].id;
            let selected = table
                .selection
                .filter(|s| s.rows().contains(&row) && s.columns().contains(&column));
            let mut node = div()
                .id(("column", id.into_raw()))
                .role(Role::GridCell)
                .aria_column_index(column)
                .aria_label(format!(
                    "{}, row {}",
                    table.data.columns.entry(column).label,
                    source + table.prefs.row_number_base
                ))
                .aria_value(text)
                .aria_selected(selected.is_some())
                .absolute()
                .left(px(left))
                .w(px(width))
                .h_full();
            if let Some(selection) = selected {
                if selection.head == cell {
                    node = node
                        .aria_active_descendant()
                        .aria_description(selection_description(selection));
                } else {
                    node = node.aria_description("Selected");
                }
            }
            for action in [AccessibleAction::Focus, AccessibleAction::Click] {
                let entity = cx.weak_entity();
                node = node.on_a11y_action(action, move |_, window, cx| {
                    _ = entity.update(cx, |table, cx| {
                        if row < table.data.row_count()
                            && table.data.source_row(row) == source
                            && table.data.columns.get(column).is_some_and(|c| c.id == id)
                        {
                            table.commit_grid_layout(cx);
                            table.selection = Some(Selection::single(cell));
                            table.reveal(cell);
                            window.focus(&table.focus, cx);
                            cx.notify();
                        }
                    });
                });
            }
            row_node = row_node.child(node);
        }
        overlay = overlay.child(row_node);
    }
    overlay.into_any_element()
}

fn selection_description(selection: Selection) -> SharedString {
    if selection.anchor == selection.head {
        return SharedString::new_static("Selected");
    }
    let rows = selection.rows();
    let columns = selection.columns();
    format!(
        "Selected display rows {} through {}, columns {} through {}",
        rows.start + 1,
        rows.end,
        columns.start + 1,
        columns.end,
    )
    .into()
}
