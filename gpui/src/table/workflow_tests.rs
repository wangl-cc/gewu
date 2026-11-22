use std::path::Path;

use gpui_kit::{
    ClipboardItem, InputEvent, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, point, px,
    test::TestWindowExt,
};

use super::ui_fixture::TableWindow;
use crate::selection::{Cell, Selection};

fn open() -> TableWindow {
    TableWindow::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv"))
}

fn single(row: u64, column: usize) -> Option<Selection> {
    Some(Selection::single(Cell { row, column }))
}

#[test]
fn only_bound_keys_reach_the_grid_and_clicks_require_cells() {
    let mut w = open();
    let normal = Modifiers::default();
    let blank_below =
        w.read(|t| t.layout.bounds.origin + point(px(t.gutter_width() + 8.), px(240.)));
    w.click(blank_below, normal);
    assert_eq!(w.selection(), None);
    w.click(w.cell(0, 0), normal);
    let blank_right =
        w.read(|t| t.layout.bounds.origin + point(px(t.layout.body_right() - 2.), px(60.)));
    w.click(blank_right, normal);
    assert_eq!(w.selection(), single(0, 0));

    w.app
        .write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
    w.keys("cmd-shift-c ctrl-cmd-f");
    assert_eq!(w.clipboard().as_deref(), Some("unchanged"));
    assert!(!w.app.read(|cx| w.table.read(cx).find.read(cx).open));
}

#[test]
fn sorted_filtered_find_and_copy_use_the_displayed_result() {
    let mut w = open();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert_eq!(w.source_rows(), [0, 2, 4]);
    // The second click arrives before the first background sort returns.
    let header = w.header(1);
    w.ui.update(|window, cx| {
        for _ in 0..2 {
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: header,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: header,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    w.settle();
    assert_eq!(w.source_rows(), [4, 2, 0]);

    w.keys("cmd-f");
    assert!(w.ui.update(|window, cx| w.table.read(cx).find.read(cx).input_focused(window, cx)));
    w.type_in_find("marker");
    assert_eq!(w.selection(), single(0, 2));
    assert_eq!(
        w.app.read(|cx| w.table.read(cx).find.read(cx).message(cx)),
        "Row 5 · note"
    );
    assert_eq!(
        w.app.read(|cx| {
            let t = w.table.read(cx);
            let id = t.data.columns[2].id;
            (0..3)
                .map(|r| t.find.read(cx).matched(id, r))
                .collect::<Vec<_>>()
        }),
        [true, true, false]
    );
    w.keys("cmd-g");
    assert_eq!(w.selection(), single(1, 2));
    w.keys("cmd-g");
    assert_eq!(w.selection(), single(0, 2));
    assert!(
        w.app
            .read(|cx| w.table.read(cx).find.read(cx).message(cx) == "Wrapped · Row 5 · note")
    );
    w.keys("cmd-shift-g");
    assert_eq!(w.selection(), single(1, 2));
    w.keys("escape");
    assert!(w.ui.update(|window, cx| w.table.read(cx).focus.is_focused(window)));

    // Only the note cell is selected. Copy captures that row and all shown columns
    // even though clearing the filter replaces the query immediately afterwards.
    w.ui.update(|window, cx| {
        window.press("cmd-c", cx);
        window.press("cmd-alt-l", cx);
    });
    w.settle();
    assert_eq!(w.clipboard().unwrap(), "keep\t5\tmarker-low\r\n");
    assert_eq!(w.source_rows(), [1, 3, 4, 2, 0]);

    w.keys("cmd-f cmd-a");
    w.type_in_find("absent");
    assert_eq!(
        w.app.read(|cx| w.table.read(cx).find.read(cx).message(cx)),
        "No matches"
    );
    w.keys("escape");
    w.click(w.header(1), Modifiers::default());
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
}

#[test]
fn find_keeps_each_navigation_request_and_filters_compose_by_column() {
    let mut w = open();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    w.click(w.cell(1, 1), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert_eq!(w.source_rows(), [2]);
    w.keys("cmd-shift-l");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l cmd-f");
    w.type_in_find("marker");
    assert_eq!(w.selection(), single(1, 2));
    w.ui.update(|window, cx| {
        for _ in 0..3 {
            window.press("cmd-g", cx);
        }
    });
    w.settle();
    assert_eq!(w.selection(), single(2, 2));
}
